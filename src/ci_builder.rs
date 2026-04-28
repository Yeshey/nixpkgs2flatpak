use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
struct State {
    last_package: Option<String>,
}

pub struct BuildCiOptions {
    pub system: String,
    pub remote: String,
    pub state_file: String,
}

pub fn run(opts: BuildCiOptions) -> Result<()> {
    let start_time = Instant::now();
    // GitHub limits runs to 6 hours. We gracefully stop at 5.5 hours to commit state.
    let max_duration = Duration::from_secs(5 * 3600 + 30 * 60);

    // 1. Load discovered.json alphabetically
    let discovered_content = fs::read_to_string("discovered.json")?;
    let discovered: HashMap<String, serde_json::Value> = serde_json::from_str(&discovered_content)?;
    let mut packages: Vec<String> = discovered.into_keys().collect();
    packages.sort();

    if packages.is_empty() {
        println!("No packages found.");
        return Ok(());
    }

    // 2. Load State
    let mut state: State = if PathBuf::from(&opts.state_file).exists() {
        serde_json::from_str(&fs::read_to_string(&opts.state_file)?)?
    } else {
        State::default()
    };

    let mut idx = 0;
    if let Some(last) = &state.last_package {
        if let Some(pos) = packages.iter().position(|p| p == last) {
            idx = (pos + 1) % packages.len(); // Pick up exactly where it left off (or wrap to 0)
        }
    }

    // 3. Mount the remote OneDrive as a local disk in the CI Runner!
    let mount_dir = PathBuf::from("/tmp/repo_mount");
    fs::create_dir_all(&mount_dir)?;

    println!("Mounting rclone remote {} to {:?}", opts.remote, mount_dir);
    let mut rclone = Command::new("rclone")
        .args([
            "mount",
            &opts.remote,
            mount_dir.to_str().unwrap(),
            "--vfs-cache-mode=full",
            "--vfs-cache-max-size=10G",
            "--vfs-write-back=5s",
        ])
        .spawn()
        .context("Failed to start rclone mount")?;

    // Wait for the mount to establish
    std::thread::sleep(Duration::from_secs(5));

    // Initialize OSTree repo on the mount if it doesn't exist
    let _ = Command::new("flatpak")
        .args(["build-init-repo", "--mode=archive-z2", mount_dir.to_str().unwrap()])
        .status();

    // 4. The Conveyor Belt Loop
    loop {
        if start_time.elapsed() > max_duration {
            println!("Time limit reached. Halting for next CI cycle.");
            break;
        }

        let pkg = &packages[idx];
        println!("\n--- Processing: {} ({}/{}) ---", pkg, idx + 1, packages.len());

        // Update state to disk before we build, in case a Nix build completely crashes the runner
        state.last_package = Some(pkg.clone());
        fs::write(&opts.state_file, serde_json::to_string_pretty(&state)?)?;

        let _ = fs::remove_dir_all("result");
        
        let build_status = Command::new("nix")
            .args(["build", &format!(".#packages.{}.{}", opts.system, pkg)])
            .status();

        if let Ok(status) = build_status {
            if status.success() {
                if let Ok(entries) = fs::read_dir("result") {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("flatpak") {
                            println!("Importing bundle directly into remote OSTree...");
                            let imp = Command::new("flatpak")
                                .args(["build-import-bundle", mount_dir.to_str().unwrap(), path.to_str().unwrap()])
                                .status();
                            
                            if imp.is_ok() && imp.unwrap().success() {
                                println!("Generating deltas and updating remote summary...");
                                let _ = Command::new("flatpak")
                                    .args(["build-update-repo", "--generate-static-deltas", mount_dir.to_str().unwrap()])
                                    .status();
                            }
                            break; 
                        }
                    }
                }
            } else {
                eprintln!("Build failed for {}. Continuing to next package.", pkg);
            }
        }

        idx = (idx + 1) % packages.len();
    }

    println!("Unmounting remote and syncing cache to OneDrive...");
    // Crucial: This gracefully tells rclone to flush the VFS cache and disconnect
    let _ = Command::new("fusermount").args(["-u", mount_dir.to_str().unwrap()]).status();
    let _ = rclone.wait();

    Ok(())
}