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

    if !skip_pull {
        println!(">>> STEP 1: Pulling repository (Safe-sync)...");
        let _ = Command::new("rclone")
            .args([
                "copy", &opts.remote, local_repo,
                "--transfers", "4", "--checkers", "8", "--tpslimit", "5",
                "--fast-list", "--size-only", "-v", "--stats", "1m"
            ])
            .status();
    }

    if !PathBuf::from(format!("{}/config", local_repo)).exists() {
        let _ = Command::new("ostree").args(["init", "--mode=archive-z2", &format!("--repo={}", local_repo)]).status();
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
        
        // Helper to construct common nix build arguments
        let run_nix = |target: &str| {
            Command::new("nix")
                .args(["build", "--impure", "-L", target])
                .env("NIXPKGS_ALLOW_UNFREE", "1")
                .env("NIXPKGS_ALLOW_BROKEN", "1")
                .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
                .status()
        };

        // --- ATTEMPT 1: Standard Build ---
        let build_result = run_nix(&format!(".#{}", pkg));

        // Determine if we need the fallback
        let final_status = match build_result {
            Ok(status) if status.success() => Ok(status),
            _ => {
                println!(">>> Standard build failed. Attempting -noicon fallback...");
                run_nix(&format!(".#{}-noicon", pkg))
            }
        };

        if let Ok(status) = final_status {
            if status.success() {
                println!(">>> Build Succeeded. Importing...");
                let _ = Command::new("bash")
                    .arg("-c")
                    .arg(format!("flatpak build-import-bundle {} result/*.flatpak", local_repo))
                    .status();

                let _ = Command::new("flatpak")
                    .args(["build-update-repo", "--generate-static-deltas", local_repo])
                    .status();

                println!(">>> Syncing to OneDrive...");
                let _ = Command::new("rclone")
                    .args([
                        "copy", local_repo, &opts.remote,
                        "--transfers", "4", "--checkers", "8", "--tpslimit", "5", "--size-only"
                    ])
                    .status();
            } else {
                eprintln!(">>> Both attempts failed for {}.", pkg);
            }
        }
        idx = (idx + 1) % packages.len();
    }
    Ok(())
}