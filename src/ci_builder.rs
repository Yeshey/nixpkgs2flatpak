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

    let skip_pull = std::env::var("SKIP_PULL").is_ok();

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
    let _ = fs::remove_dir_all(local_repo);
    fs::create_dir_all(local_repo)?;

    if skip_pull {
        println!(">>> SKIP_PULL is set. Starting with a fresh local repository.");
    } else {
        println!(">>> STEP 1: Pulling repository from OneDrive...");
        let _ = Command::new("rclone")
            .args([
                "copy", &opts.remote, local_repo,
                "--transfers", "8",
                "--checkers", "8",
                "--tpslimit", "5",
                "--fast-list",
                "--size-only",
                "-v",
                "--stats", "1m"
            ])
            .status();
    }

    // Always ensure the repo is initialized locally
    if !PathBuf::from(format!("{}/config", local_repo)).exists() {
        println!(">>> Initializing local OSTree repo...");
        let _ = Command::new("ostree").args(["init", "--mode=archive-z2", &format!("--repo={}", local_repo)]).status();
    }
    
    // ── NEW: SELF-HEALING LOGIC ──
    // If objects exist but the summary is missing (because you deleted it), fix it immediately!
    if !PathBuf::from(format!("{}/summary", local_repo)).exists() && PathBuf::from(format!("{}/objects", local_repo)).exists() {
        println!(">>> Summary missing but objects found. Repairing repository...");
        let _ = Command::new("flatpak").args(["build-update-repo", "--generate-static-deltas", local_repo]).status();
        
        println!(">>> Pushing repaired summary back to OneDrive...");
        let _ = Command::new("rclone")
            .args(["copy", local_repo, &opts.remote, "--transfers", "4", "--tpslimit", "5", "--size-only"])
            .status();
    }

    let _ = Command::new("ostree").args(["config", "--repo", local_repo, "set", "core.min-free-space-percent", "0"]).status();

    println!(">>> STEP 2: Starting build loop...");
    loop {
        if start_time.elapsed() > max_duration { break; }

        let pkg = &packages[idx];
        println!("\n>>> [ {}/{} ] Building: {}", idx + 1, packages.len(), pkg);

        state.last_package = Some(pkg.clone());
        fs::write(&opts.state_file, serde_json::to_string_pretty(&state)?)?;

        let _ = fs::remove_dir_all("result");
        
        // --- ATTEMPT 1: Standard Build ---
        let mut build_status = Command::new("nix")
            .args(["build", "--impure", "-L", &format!(".#{}", pkg)])
            .env("NIXPKGS_ALLOW_UNFREE", "1")
            .env("NIXPKGS_ALLOW_BROKEN", "1")
            .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
            .status();

        // --- ATTEMPT 2: Fallback (No Icon) ---
        if build_status.is_err() || !build_status.unwrap().success() {
            println!(">>> Standard build failed. Attempting -noicon fallback...");
            build_status = Command::new("nix")
                .args(["build", "--impure", "-L", &format!(".#{}-noicon", pkg)])
                .env("NIXPKGS_ALLOW_UNFREE", "1")
                .env("NIXPKGS_ALLOW_BROKEN", "1")
                .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
                .status();
        }

        if let Ok(status) = build_status {
            if status.success() {
                println!(">>> Build Succeeded. Importing...");
                
                // Use shell to expand the glob safely
                let _ = Command::new("bash")
                    .arg("-c")
                    .arg(format!("flatpak build-import-bundle {} result/*.flatpak", local_repo))
                    .status();

                let _ = Command::new("flatpak")
                    .args(["build-update-repo", "--generate-static-deltas", local_repo])
                    .status();

                println!(">>> Syncing back to OneDrive...");
                let _ = Command::new("rclone")
                    .args([
                        "copy", local_repo, &opts.remote, 
                        "--transfers", "4", "--checkers", "8", "--tpslimit", "5", 
                        "--fast-list", "--size-only"
                    ])
                    .status();
            } else {
                eprintln!(">>> Both attempts failed for {}. Skipping.", pkg);
            }
        }
        idx = (idx + 1) % packages.len();
    }
    Ok(())
}