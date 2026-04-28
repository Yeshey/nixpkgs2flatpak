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

    println!("Pulling repo from OneDrive...");
    let _ = Command::new("rclone")
        .args(["copy", &opts.remote, local_repo, "--transfers", "16", "--fast-list"])
        .status();

    if !PathBuf::from(format!("{}/config", local_repo)).exists() {
        let _ = Command::new("ostree").args(["init", "--mode=archive-z2", &format!("--repo={}", local_repo)]).status();
    }
    
    // Safety: Disable free space check and try to prune any broken dangling refs
    let _ = Command::new("ostree").args(["config", "--repo", local_repo, "set", "core.min-free-space-percent", "0"]).status();

    loop {
        if start_time.elapsed() > max_duration { break; }

        let pkg = &packages[idx];
        println!("\n--- [ {}/{} ] Building: {} ---", idx + 1, packages.len(), pkg);

        state.last_package = Some(pkg.clone());
        fs::write(&opts.state_file, serde_json::to_string_pretty(&state)?)?;

        let _ = fs::remove_dir_all("result");
        let build_status = Command::new("nix")
            .args(["build", "--impure", &format!(".#{}", pkg)])
            .env("NIXPKGS_ALLOW_UNFREE", "1")
            .env("NIXPKGS_ALLOW_BROKEN", "1")
            .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
            .status();

        if let Ok(status) = build_status {
            if status.success() {
                println!("Importing bundle...");
                // Attempt to import. If this fails due to corruption, we try to repair the repo summary.
                let imp = Command::new("bash")
                    .arg("-c")
                    .arg(format!("flatpak build-import-bundle {} result/*.flatpak", local_repo))
                    .status();

                if imp.is_ok() && imp.unwrap().success() {
                    // Update repo. If this crashes, the repo was corrupted.
                    let update = Command::new("flatpak")
                        .args(["build-update-repo", "--generate-static-deltas", local_repo])
                        .status();
                    
                    if update.is_err() || !update.unwrap().success() {
                        eprintln!("Corruption detected! Attempting to prune broken refs...");
                        let _ = Command::new("ostree").args(["prune", &format!("--repo={}", local_repo)]).status();
                    } else {
                        println!("Syncing to OneDrive...");
                        // Only push if the repo is in a healthy state
                        let _ = Command::new("rclone")
                            .args(["copy", local_repo, &opts.remote, "--transfers", "16", "--fast-list"])
                            .status();
                    }
                }
            }
        }
        idx = (idx + 1) % packages.len();
    }
    Ok(())
}