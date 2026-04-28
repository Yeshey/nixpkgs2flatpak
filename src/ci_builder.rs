use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
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
}

/// The filename used for the state file, both locally and inside the remote.
fn state_filename(system: &str) -> String {
    format!("state-{}.json", system)
}

/// Download the state file from OneDrive into the working directory.
/// Returns a default State if the remote file doesn't exist yet.
fn pull_state(remote: &str, system: &str) -> State {
    let filename = state_filename(system);
    // "remote:path/file" → rclone copies it to the local filename
    let remote_path = format!("{}/{}", remote, filename);
    let status = Command::new("rclone")
        .args(["copyto", &remote_path, &filename, "--retries", "3"])
        .status();

    match status {
        Ok(s) if s.success() => {
            match fs::read_to_string(&filename).ok().and_then(|c| serde_json::from_str(&c).ok()) {
                Some(state) => {
                    println!(">>> Resumed state from OneDrive ({}).", filename);
                    state
                }
                None => {
                    println!(">>> State file present but unreadable; starting fresh.");
                    State::default()
                }
            }
        }
        _ => {
            println!(">>> No previous state found on OneDrive; starting from the beginning.");
            State::default()
        }
    }
}

/// Upload the state file from the working directory to OneDrive.
/// Called after every package so progress is never lost, even on forced cancellation.
fn push_state(remote: &str, system: &str) {
    let filename = state_filename(system);
    let remote_path = format!("{}/{}", remote, filename);
    let status = Command::new("rclone")
        .args(["copyto", &filename, &remote_path, "--retries", "3"])
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        eprintln!("!!! WARNING: failed to push state to OneDrive. Progress may be lost if the runner is killed now.");
    }
}

pub fn run(opts: BuildCiOptions) -> Result<()> {
    let start_time = Instant::now();
    let max_duration = Duration::from_secs(5 * 3600 + 30 * 60);

    let discovered_content = fs::read_to_string("discovered.json")?;
    let discovered: BTreeMap<String, serde_json::Value> = serde_json::from_str(&discovered_content)?;
    let mut packages: Vec<String> = discovered.into_keys().collect();
    packages.sort();

    if packages.is_empty() { return Ok(()); }

    // ── Restore position from OneDrive ────────────────────────────────────────
    let state = pull_state(&opts.remote, &opts.system);

    let mut idx = 0;
    if let Some(last) = &state.last_package {
        if let Some(pos) = packages.iter().position(|p| p == last) {
            idx = (pos + 1) % packages.len();
            println!(">>> Resuming after '{}' (index {}).", last, idx);
        } else {
            println!(">>> Last package '{}' not found in current list; starting from the beginning.", last);
        }
    }

    let local_repo = "local_repo";

    if !Path::new(local_repo).join("objects").exists() {
        println!(">>> Initializing local OSTree repo...");
        fs::create_dir_all(local_repo)?;
        let _ = Command::new("ostree")
            .args(["init", "--mode=archive-z2", &format!("--repo={}", local_repo)])
            .status();
    }

    let _ = Command::new("ostree")
        .args(["config", "--repo", local_repo, "set", "core.min-free-space-percent", "0"])
        .status();

    loop {
        if start_time.elapsed() > max_duration {
            println!(">>> Time limit reached. Stopping.");
            break;
        }

        let pkg = packages[idx].clone();
        println!("\n>>> [ {}/{} ] Building: {}", idx + 1, packages.len(), pkg);

        let _ = fs::remove_dir_all("result");

        let run_nix = |target: &str| {
            Command::new("nix")
                .args(["build", "--impure", "-L", target])
                .env("NIXPKGS_ALLOW_UNFREE", "1")
                .env("NIXPKGS_ALLOW_BROKEN", "1")
                .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
                .status()
        };

        let build_result = run_nix(&format!(".#{}", pkg));
        let final_status = match build_result {
            Ok(s) if s.success() => Ok(s),
            _ => {
                println!(">>> Standard build failed. Attempting -fixed icon fallback...");
                run_nix(&format!(".#{}-fixed", pkg))
            }
        };

        if let Ok(status) = final_status {
            if status.success() {
                println!(">>> Build succeeded. Importing into local repo...");
                let _ = Command::new("bash")
                    .arg("-c")
                    .arg(format!("flatpak build-import-bundle {} result/*.flatpak", local_repo))
                    .status();

                let _ = Command::new("flatpak")
                    .args(["build-update-repo", "--generate-static-deltas", local_repo])
                    .status();

                println!(">>> Uploading new objects to OneDrive...");
                let _ = Command::new("rclone")
                    .args([
                        "copy", local_repo, &opts.remote,
                        "--transfers", "4", "--checkers", "8", "--tpslimit", "5",
                        "--fast-list", "--size-only",
                    ])
                    .status();
            }
        }

        // ── Persist progress immediately after every package ──────────────────
        // Write to a temp file then rename so the file is never half-written.
        let new_state = State { last_package: Some(pkg.clone()) };
        let filename = state_filename(&opts.system);
        let tmp_filename = format!("{}.tmp", filename);
        if let Ok(json) = serde_json::to_string_pretty(&new_state) {
            if fs::write(&tmp_filename, &json).is_ok() {
                let _ = fs::rename(&tmp_filename, &filename);
            }
        }
        push_state(&opts.remote, &opts.system);

        idx = (idx + 1) % packages.len();
    }
    Ok(())
}