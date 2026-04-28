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
    let max_duration = Duration::from_secs(5 * 3600 + 30 * 60);

    let discovered_content = fs::read_to_string("discovered.json")?;
    let discovered: HashMap<String, serde_json::Value> = serde_json::from_str(&discovered_content)?;
    let mut packages: Vec<String> = discovered.into_keys().collect();
    packages.sort();

    if packages.is_empty() { return Ok(()); }

    let mut state: State = if PathBuf::from(&opts.state_file).exists() {
        serde_json::from_str(&fs::read_to_string(&opts.state_file)?)?
    } else {
        State::default()
    };

    let mut idx = 0;
    if let Some(last) = &state.last_package {
        if let Some(pos) = packages.iter().position(|p| p == last) {
            idx = (pos + 1) % packages.len();
        }
    }

    let local_repo = "/tmp/local_repo";
    let _ = fs::remove_dir_all(local_repo); // Start fresh to save CI disk space
    fs::create_dir_all(local_repo)?;

    println!("Pulling current repository state from OneDrive...");
    let _ = Command::new("rclone")
        .args(["copy", &opts.remote, local_repo, "--transfers", "16", "--checkers", "16"])
        .status();

    if !PathBuf::from(format!("{}/config", local_repo)).exists() {
        println!("Initializing new OSTree repo...");
        let _ = Command::new("ostree")
            .args(["init", "--mode=archive-z2", &format!("--repo={}", local_repo)])
            .status();
    }

    // CRITICAL: Disable the 3% free space safety check which kills builds on GitHub
    let _ = Command::new("ostree")
        .args(["config", "--repo", local_repo, "set", "core.min-free-space-percent", "0"])
        .status();

    loop {
        if start_time.elapsed() > max_duration {
            println!("Time limit reached.");
            break;
        }

        let pkg = &packages[idx];
        println!("\n--- Processing: {} ({}/{}) ---", pkg, idx + 1, packages.len());

        state.last_package = Some(pkg.clone());
        fs::write(&opts.state_file, serde_json::to_string_pretty(&state)?)?;

        let _ = fs::remove_dir_all("result");
        
        // Pass env vars for unfree/broken and use --impure
        let build_status = Command::new("nix")
            .args(["build", "--impure", &format!(".#{}", pkg)])
            .env("NIXPKGS_ALLOW_UNFREE", "1")
            .env("NIXPKGS_ALLOW_BROKEN", "1")
            .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
            .status();

        if let Ok(status) = build_status {
            if status.success() {
                if let Ok(entries) = fs::read_dir("result") {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("flatpak") {
                            println!("Importing bundle...");
                            let imp = Command::new("flatpak")
                                .args(["build-import-bundle", local_repo, path.to_str().unwrap()])
                                .status();
                            
                            if imp.is_ok() && imp.unwrap().success() {
                                println!("Updating repo metadata...");
                                let _ = Command::new("flatpak")
                                    .args(["build-update-repo", "--generate-static-deltas", local_repo])
                                    .status();

                                println!("Syncing to OneDrive...");
                                let _ = Command::new("rclone")
                                    .args(["copy", local_repo, &opts.remote, "--transfers", "16"])
                                    .status();
                            }
                            break; 
                        }
                    }
                }
            } else {
                eprintln!("Build failed for {}.", pkg);
            }
        }

        idx = (idx + 1) % packages.len();
    }

    Ok(())
}