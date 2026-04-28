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
    // Stop at 5.5 hours to allow final sync and git commit
    let max_duration = Duration::from_secs(5 * 3600 + 30 * 60);

    let discovered_content = fs::read_to_string("discovered.json")?;
    let discovered: HashMap<String, serde_json::Value> = serde_json::from_str(&discovered_content)?;
    let mut packages: Vec<String> = discovered.into_keys().collect();
    packages.sort();

    if packages.is_empty() { 
        println!("Error: No packages found in discovered.json");
        return Ok(()); 
    }

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
    let _ = fs::remove_dir_all(local_repo); 
    fs::create_dir_all(local_repo)?;

    println!(">>> STEP 1: Pulling current repository state from OneDrive...");
    // Added -v for verbosity, --stats to show progress every minute, and --size-only for speed
    let _ = Command::new("rclone")
        .args([
            "copy", &opts.remote, local_repo, 
            "--transfers", "16", 
            "--checkers", "16", 
            "--fast-list", 
            "--size-only",
            "-v", 
            "--stats", "1m"
        ])
        .status();

    if !PathBuf::from(format!("{}/config", local_repo)).exists() {
        println!(">>> Repo not found on remote. Initializing new OSTree repo...");
        let _ = Command::new("ostree").args(["init", "--mode=archive-z2", &format!("--repo={}", local_repo)]).status();
    }
    
    let _ = Command::new("ostree").args(["config", "--repo", local_repo, "set", "core.min-free-space-percent", "0"]).status();

    println!(">>> STEP 2: Starting Conveyor Belt loop...");
    loop {
        let elapsed = start_time.elapsed();
        if elapsed > max_duration { 
            println!(">>> Time limit reached ({:?}). Shutting down loop.", elapsed);
            break; 
        }

        let pkg = &packages[idx];
        println!("\n========================================================");
        println!(">>> [ {}/{} ] Building: {}", idx + 1, packages.len(), pkg);
        println!("========================================================");

        state.last_package = Some(pkg.clone());
        fs::write(&opts.state_file, serde_json::to_string_pretty(&state)?)?;

        let _ = fs::remove_dir_all("result");
        
        // Added -L to Nix to print build logs in real-time to GitHub
        let build_status = Command::new("nix")
            .args(["build", "--impure", "-L", &format!(".#{}", pkg)])
            .env("NIXPKGS_ALLOW_UNFREE", "1")
            .env("NIXPKGS_ALLOW_BROKEN", "1")
            .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
            .status();

        if let Ok(status) = build_status {
            if status.success() {
                println!(">>> Build Succeeded. Importing into OSTree...");
                
                let imp = Command::new("bash")
                    .arg("-c")
                    .arg(format!("flatpak build-import-bundle {} result/*.flatpak", local_repo))
                    .status();

                if imp.is_ok() && imp.unwrap().success() {
                    println!(">>> Regenerating static deltas and summary...");
                    let _ = Command::new("flatpak")
                        .args(["build-update-repo", "--generate-static-deltas", local_repo])
                        .status();

                    println!(">>> Syncing new objects to OneDrive...");
                    // Using -v and --size-only here too
                    let _ = Command::new("rclone")
                        .args([
                            "copy", local_repo, &opts.remote, 
                            "--transfers", "16", 
                            "--checkers", "16", 
                            "--fast-list", 
                            "--size-only",
                            "-v",
                            "--stats", "1m"
                        ])
                        .status();
                } else {
                    eprintln!(">>> Error: Flatpak import failed. Possible repo corruption.");
                }
            } else {
                eprintln!(">>> Build failed for {}. Skipping to next.", pkg);
            }
        }
        idx = (idx + 1) % packages.len();
    }
    
    println!(">>> Conveyor Belt cycle finished for this run.");
    Ok(())
}