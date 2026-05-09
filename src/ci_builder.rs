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

#[derive(Serialize, Deserialize, Clone)]
struct PkgMeta {
    #[serde(rename = "appId")]
    app_id: String,
    runtime: String,
    #[serde(rename = "isCurated")]
    is_curated: bool,
}

pub struct BuildCiOptions {
    pub system: String,
    pub remote: String,
    /// Which of the 7 parallel runners this process is. Runners 1-6 each own a
    /// contiguous slice of the aa-zz two-letter namespace; runner 7 handles every
    /// package whose first two characters are not both ASCII lowercase letters
    /// (prefixes like `_0`, `a-`, `z8`, single-char names, etc.).
    pub runner_id: u8,
}

// ── PARTITION HELPERS ────────────────────────────────────────────────────────

/// Returns which runner (1-7) owns a given package name, based on its first
/// two characters.
///
/// The 676 two-letter combos (aa-zz) are split into 6 equal slices:
///   runner 1: aa–eh  | runner 2: ei–iq  | runner 3: ir–mz
///   runner 4: na–rh  | runner 5: ri–vq  | runner 6: vr–zz
///   runner 7: anything that doesn't start with two lowercase ASCII letters
fn runner_for_package(name: &str) -> u8 {
    let mut chars = name.chars();
    let c0 = match chars.next() {
        Some(c) if c.is_ascii_lowercase() => c,
        _ => return 7,
    };
    let c1 = match chars.next() {
        Some(c) if c.is_ascii_lowercase() => c,
        _ => return 7,
    };
    let prefix = [c0, c1];
    if      prefix <= ['e', 'h'] { 1 }
    else if prefix <= ['i', 'q'] { 2 }
    else if prefix <= ['m', 'z'] { 3 }
    else if prefix <= ['r', 'h'] { 4 }
    else if prefix <= ['v', 'q'] { 5 }
    else                         { 6 }
}

/// Maps a `system` string to the short architecture folder name used under
/// `failed_apps/` on both the local disk and OneDrive.
fn arch_folder(system: &str) -> &'static str {
    if system.starts_with("x86") { "x86" } else { "aarch64" }
}

// ── STATE PERSISTENCE ────────────────────────────────────────────────────────

/// Each runner keeps its own state file so runners never step on one another.
/// Files live in `github_runners_state/` both locally and on OneDrive.
fn state_filename(system: &str, runner_id: u8) -> String {
    format!("github_runners_state/state-{}-runner{}.json", system, runner_id)
}

fn pull_state(remote: &str, system: &str, runner_id: u8) -> State {
    let filename = state_filename(system, runner_id);
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

fn push_state(remote: &str, system: &str, runner_id: u8) {
    let filename = state_filename(system, runner_id);
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
    
    // --- TIME LIMIT: 5 Hours and 20 Minutes ---
    let max_duration = Duration::from_secs(5 * 3600 + 20 * 60);

    println!(">>> Fetching final package metadata from Nix...");
    let meta_status = Command::new("nix")
        .args(["build", ".#ci-metadata", "-o", "ci-metadata-result", "--impure"])
        .status()?;
    
    if !meta_status.success() {
        anyhow::bail!("Failed to evaluate ci-metadata from Nix.");
    }

    let metadata_content = fs::read_to_string("ci-metadata-result")?;
    let metadata: BTreeMap<String, PkgMeta> = serde_json::from_str(&metadata_content)?;
    
    let mut packages: Vec<String> = metadata.keys().cloned().collect();

    // Sort: Curated apps FIRST, then alphabetically
    packages.sort_by(|a, b| {
        let meta_a = &metadata[a];
        let meta_b = &metadata[b];
        match meta_b.is_curated.cmp(&meta_a.is_curated) {
            std::cmp::Ordering::Equal => a.cmp(b),
            other => other,
        }
    });

    // ── RUNNER PARTITION ──────────────────────────────────────────────────────
    // Each runner only processes its own slice of the package namespace so
    // runners never duplicate work and state bookmarks never conflict.
    packages.retain(|p| runner_for_package(p) == opts.runner_id);
    println!(">>> Runner {}: {} packages assigned.", opts.runner_id, packages.len());

    // SINGLE PACKAGE OVERRIDE
    let single_pkg = std::env::var("TARGET_PACKAGE").ok();
    if let Some(target) = &single_pkg {
        if packages.contains(target) {
            packages = vec![target.clone()];
            println!(">>> TARGET_PACKAGE set. Only building '{}'.", target);
        } else {
            println!("!!! Target package '{}' not found in this runner's partition. It might belong to a different runner, or not exist in Nixpkgs.", target);
            return Ok(());
        }
    }

    if packages.is_empty() { return Ok(()); }

    let state = pull_state(&opts.remote, &opts.system, opts.runner_id);
    let mut idx = 0;
    
    // Only resume from state if we are doing the endless loop
    if single_pkg.is_none() {
        if let Some(last) = &state.last_package {
            if let Some(pos) = packages.iter().position(|p| p == last) {
                idx = (pos + 1) % packages.len();
                println!(">>> Resuming after '{}' (index {}).", last, idx);
            } else {
                println!(">>> Last package '{}' not found in current list; starting from the beginning.", last);
            }
        }
    }

    println!(">>> Ensuring Flathub remote is configured to pull runtimes for testing...");
    let _ = Command::new("flatpak")
        .args(["--user", "remote-add", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"])
        .status();

    // Tracker for Garbage Collection batching
    let mut packages_since_gc = 0;

    loop {
        // Stop exactly at 5h 20m.
        if start_time.elapsed() > max_duration {
            println!(">>> Time limit (5h 20m) reached. Stopping gracefully to avoid GitHub Force Kill.");
            break;
        }

        let pkg = packages[idx].clone();
        let is_curated = metadata[&pkg].is_curated;
        let tag = if is_curated { "[CURATED]" } else { "[AUTO]" };
        println!("\n>>> [ {}/{} ] {} Building: {}", idx + 1, packages.len(), tag, pkg);

        // ─── DISK SPACE MANAGEMENT ───
        let local_repo = "local_repo";
        let _ = fs::remove_dir_all("result");
        let _ = fs::remove_dir_all(local_repo);

        if single_pkg.is_none() {
            // Only run GC every 5 packages to save massive amounts of time
            if packages_since_gc >= 5 {
                println!(">>> 5 packages built! Running Nix garbage collection to free up disk space...");
                let _ = Command::new("nix-store").arg("--gc").status();
                packages_since_gc = 0;
            } else {
                packages_since_gc += 1;
            }
        }

        println!(">>> Initializing clean local OSTree repo for this package...");
        let _ = fs::create_dir_all(local_repo);
        let _ = Command::new("ostree")
            .args(["init", "--mode=archive-z2", &format!("--repo={}", local_repo)])
            .status();
        let _ = Command::new("ostree")
            .args(["config", "--repo", local_repo, "set", "core.min-free-space-percent", "0"])
            .status();

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

        let app_id = &metadata[&pkg].app_id;

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

                    // --noninteractive forces Flatpak to auto-download required runtimes from Flathub
                    let _ = Command::new("flatpak")
                        .args(["--user", "install", "--noninteractive", "-y", "test_repo", app_id])
                        .status();

                    // RUN THE TEST AND CAPTURE THE OUTPUT (stdout & stderr)
                    let test_output = Command::new("xvfb-run")
                        .args([
                            "-a", 
                            "-s", "-screen 0 1024x768x24 +extension GLX", 
                            "timeout", "10", 
                            "flatpak", "run", 
                            "--env=LIBGL_ALWAYS_SOFTWARE=1", 
                            "--env=GALLIUM_DRIVER=llvmpipe", 
                            app_id
                        ])
                        .output();

                    let _ = Command::new("flatpak")
                        .args(["--user", "uninstall", "--noninteractive", "-y", app_id])
                        .status();

                    // Evaluate test results
                    let (passed, output_text) = match test_output {
                        Ok(out) => {
                            let code = out.status.code().unwrap_or(0);
                            // 0 = Clean exit, 124 = Time ran out, 137/143 = Terminated
                            let p = code == 0 || code == 124 || code == 137 || code == 143;
                            
                            let mut text = String::from_utf8_lossy(&out.stdout).to_string();
                            text.push_str("\n--- STDERR ---\n");
                            text.push_str(&String::from_utf8_lossy(&out.stderr));
                            (p, text)
                        },
                        Err(e) => (false, format!("Failed to execute xvfb-run: {}", e)),
                    };

                    if !passed {
                        println!("!!! TEST FAILED: {} crashed upon launch. Skipping upload.", app_id);

                        // Each runner writes its own shard file so concurrent runners never
                        // race on the same file.  A dedicated merge job in CI combines all
                        // shards into the final failed_apps_{system}.txt after the run.
                        let arch = arch_folder(&opts.system);
                        let runner_failed_file = format!("failed_apps/{}/runner{}.txt", arch, opts.runner_id);
                        let remote_failed_file = format!("{}/failed_apps/{}/runner{}.txt", opts.remote, arch, opts.runner_id);

                        // 1. Download this runner's existing shard from OneDrive (accumulates across days)
                        let _ = Command::new("rclone")
                            .args(["copyto", &remote_failed_file, &runner_failed_file, "--retries", "3"])
                            .status();

                        // 2. Append to local shard
                        let _ = fs::create_dir_all(format!("failed_apps/{}", arch));
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&runner_failed_file) {
                            let _ = writeln!(f, "{} ({})", pkg, app_id);
                        }
                        
                        // 3. Upload updated shard back to OneDrive
                        let _ = Command::new("rclone")
                            .args(["copyto", &runner_failed_file, &remote_failed_file, "--retries", "3"])
                            .status();

                        // 4. Save the detailed crash log locally
                        let _ = fs::create_dir_all("failed_logs");
                        let log_filename = format!("failed_logs/{}_{}.log", app_id, opts.system);
                        let _ = fs::write(&log_filename, &output_text);

                        // 5. Upload detailed crash log to the failed_apps folder on OneDrive
                        let _ = Command::new("rclone")
                            .args(["copyto", &log_filename, &format!("{}/failed_apps/{}_{}.log", opts.remote, app_id, opts.system)])
                            .status();

                        if single_pkg.is_some() {
                            println!(">>> Single package test complete. Exiting without altering state bookmark.");
                            break; 
                        }

                        let new_state = State { last_package: Some(pkg.clone()) };
                        let filename = state_filename(&opts.system, opts.runner_id);
                        let tmp_filename = format!("{}.tmp", filename);
                        let _ = fs::create_dir_all("github_runners_state");
                        if let Ok(json) = serde_json::to_string_pretty(&new_state) {
                            if fs::write(&tmp_filename, &json).is_ok() {
                                let _ = fs::rename(&tmp_filename, &filename);
                            }
                        }
                        push_state(&opts.remote, &opts.system, opts.runner_id);
                        
                        idx = (idx + 1) % packages.len();
                        continue;
                    } else {
                        println!(">>> Test passed! App successfully stayed alive.");
                    }
                }

                println!(">>> Uploading new objects to OneDrive...");
                let objects_status = Command::new("rclone")
                    .args([
                        "copy", &format!("{}/objects", local_repo), &format!("{}/objects", opts.remote),
                        "--transfers", "4", "--checkers", "8", "--tpslimit", "5",
                        "--fast-list", "--size-only",
                    ])
                    .status();

                if objects_status.map_or(false, |s| s.success()) {
                    let _ = Command::new("rclone")
                        .args([
                            "copy", local_repo, &opts.remote,
                            "--transfers", "4", "--checkers", "8", "--tpslimit", "5",
                            "--fast-list", "--size-only",
                            "--exclude", "/objects/**",
                            "--exclude", "summary",
                            "--exclude", "summary.sig",
                        ])
                        .status();
                } else {
                    println!(">>> Warning: Failed to upload objects. Skipping refs upload to prevent remote corruption.");
                }
            }
        }

        if single_pkg.is_some() {
            println!(">>> Single package test complete. Exiting without altering state bookmark.");
            break; 
        }

        let new_state = State { last_package: Some(pkg.clone()) };
        let filename = state_filename(&opts.system, opts.runner_id);
        let tmp_filename = format!("{}.tmp", filename);
        let _ = fs::create_dir_all("github_runners_state");
        if let Ok(json) = serde_json::to_string_pretty(&new_state) {
            if fs::write(&tmp_filename, &json).is_ok() {
                let _ = fs::rename(&tmp_filename, &filename);
            }
        }
        push_state(&opts.remote, &opts.system, opts.runner_id);

        idx = (idx + 1) % packages.len();
    }
    Ok(())
}