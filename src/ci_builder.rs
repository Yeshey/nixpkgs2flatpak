use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
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

fn state_filename(system: &str) -> String {
    format!("state-{}.json", system)
}

fn pull_state(remote: &str, system: &str) -> State {
    let filename = state_filename(system);
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
    let mut packages: Vec<String> = discovered.keys().cloned().collect();
    packages.sort();

    if packages.is_empty() { return Ok(()); }

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

        let app_id = discovered.get(&pkg)
            .and_then(|v| v.get("appId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if let Ok(status) = final_status {
            if status.success() {
                println!(">>> Build succeeded. Importing into local repo...");
                let _ = Command::new("bash")
                    .arg("-c")
                    .arg(format!("flatpak build-import-bundle {} result/*.flatpak", local_repo))
                    .status();

                // ── TESTING PHASE ──
                if !app_id.is_empty() {
                    println!(">>> Testing application launch on virtual display for {}...", app_id);

                    let _ = Command::new("flatpak")
                        .args(["--user", "remote-add", "--no-gpg-verify", "--if-not-exists", "test_repo", local_repo])
                        .status();

                    let _ = Command::new("flatpak")
                        .args(["--user", "install", "--noninteractive", "-y", "test_repo", &app_id])
                        .status();

                    // Run the app with a 5-second timeout on a fake X11 display
                    let test_status = Command::new("xvfb-run")
                        .args(["-a", "timeout", "5", "flatpak", "run", &app_id])
                        .status();

                    let _ = Command::new("flatpak")
                        .args(["--user", "uninstall", "--noninteractive", "-y", &app_id])
                        .status();

                    let passed = match test_status {
                        Ok(s) => {
                            let code = s.code().unwrap_or(0);
                            // 0 = Clean exit, 124 = Time ran out (it stayed alive!), 137/143 = Terminated
                            code == 0 || code == 124 || code == 137 || code == 143
                        },
                        Err(_) => false,
                    };

                    if !passed {
                        println!("!!! TEST FAILED: {} crashed upon launch. Skipping upload.", app_id);
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("failed_apps.log") {
                            let _ = writeln!(f, "{} ({})", pkg, app_id);
                        }
                        
                        // Push failure log to OneDrive
                        let _ = Command::new("rclone")
                            .args(["copyto", "failed_apps.log", &format!("{}/failed_apps.log", opts.remote)])
                            .status();

                        // Advance state, skip rclone upload, move to next package
                        let new_state = State { last_package: Some(pkg.clone()) };
                        let filename = state_filename(&opts.system);
                        let tmp_filename = format!("{}.tmp", filename);
                        if let Ok(json) = serde_json::to_string_pretty(&new_state) {
                            if fs::write(&tmp_filename, &json).is_ok() {
                                let _ = fs::rename(&tmp_filename, &filename);
                            }
                        }
                        push_state(&opts.remote, &opts.system);
                        
                        if std::env::var("CI_SINGLE_PACKAGE").is_ok() { break; }
                        idx = (idx + 1) % packages.len();
                        continue;
                    } else {
                        println!(">>> Test passed! App successfully stayed alive.");
                    }
                }

                println!(">>> Uploading new objects to OneDrive...");
                
                // 1. Upload objects first. It's safe if interrupted because objects are content-addressed.
                let objects_status = Command::new("rclone")
                    .args([
                        "copy", &format!("{}/objects", local_repo), &format!("{}/objects", opts.remote),
                        "--transfers", "4", "--checkers", "8", "--tpslimit", "5",
                        "--fast-list", "--size-only",
                    ])
                    .status();

                if objects_status.map_or(false, |s| s.success()) {
                    // 2. Upload refs only AFTER objects are fully present on the remote.
                    let _ = Command::new("rclone")
                        .args([
                            "copy", local_repo, &opts.remote,
                            "--transfers", "4", "--checkers", "8", "--tpslimit", "5",
                            "--fast-list", "--size-only",
                            "--exclude", "/objects/**",
                            // Never overwrite the server's authoritative summary files.
                            "--exclude", "summary",
                            "--exclude", "summary.sig",
                        ])
                        .status();
                } else {
                    println!(">>> Warning: Failed to upload objects. Skipping refs upload to prevent remote corruption.");
                }
            }
        }

        let new_state = State { last_package: Some(pkg.clone()) };
        let filename = state_filename(&opts.system);
        let tmp_filename = format!("{}.tmp", filename);
        if let Ok(json) = serde_json::to_string_pretty(&new_state) {
            if fs::write(&tmp_filename, &json).is_ok() {
                let _ = fs::rename(&tmp_filename, &filename);
            }
        }
        push_state(&opts.remote, &opts.system);

        // Escape hatch if we only wanted to test a single package
        if std::env::var("CI_SINGLE_PACKAGE").is_ok() { break; }
        idx = (idx + 1) % packages.len();
    }
    Ok(())
}