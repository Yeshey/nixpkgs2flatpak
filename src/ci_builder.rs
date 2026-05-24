use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
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
    /// Which of the 2 parallel runners this process is (1 or 2).
    /// Runner 1 works forward from the start of the sorted list (a…);
    /// runner 2 works backward from the end (…z). They converge in the middle.
    pub runner_id: u8,
    /// How many minutes to run before stopping gracefully. Defaults to 320 (5h 20m).
    pub max_minutes: f64,
}

// ── PARTITION HELPERS ────────────────────────────────────────────────────────

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

    // Hard 6h ceiling — matches GitHub's job limit.
    // The 3:15h "don't start new builds" guard below ensures we always have
    // enough budget to finish a 3h build + upload before hitting this wall.
    let max_duration = Duration::from_secs(360 * 60);
    // Per-package build timeout: 3 hours.
    let limit_3h = 10800u64;
    println!(">>> Strict time limit: 6 hours (360 minutes).");

    println!(">>> Fetching final package metadata from Nix...");
    // A failure here must never mark the job as failed — it just means this
    // run produced nothing. Exit 0 so GitHub doesn't send a failure email.
    let meta_status = match Command::new("nix")
        .args(["build", ".#ci-metadata", "-o", "ci-metadata-result", "--impure"])
        .status()
    {
        Ok(s) => s,
        Err(e) => { eprintln!("!!! Could not spawn nix: {}. Exiting cleanly.", e); return Ok(()); }
    };
    if !meta_status.success() {
        eprintln!("!!! Failed to evaluate ci-metadata from Nix. Exiting cleanly.");
        return Ok(());
    }

    let metadata_content = match fs::read_to_string("ci-metadata-result") {
        Ok(c) => c,
        Err(e) => { eprintln!("!!! Could not read ci-metadata-result: {}. Exiting cleanly.", e); return Ok(()); }
    };
    let metadata: BTreeMap<String, PkgMeta> = match serde_json::from_str(&metadata_content) {
        Ok(m) => m,
        Err(e) => { eprintln!("!!! Could not parse ci-metadata JSON: {}. Exiting cleanly.", e); return Ok(()); }
    };

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
    // Split the sorted list in half by position.
    // Runner 1 works forward from the start (a...).
    // Runner 2 works backward from the end (...z), so they converge in the middle
    // and both halves get covered even if a runner is cancelled early.
    let mid = (packages.len() + 1) / 2; // runner 1 gets the ceiling half
    match opts.runner_id {
        1 => packages.truncate(mid),
        2 => { packages.drain(..mid); packages.reverse(); }
        _ => { eprintln!("!!! Unknown runner_id {}. Valid values: 1, 2.", opts.runner_id); return Ok(()); }
    }
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
        let remaining = max_duration.saturating_sub(start_time.elapsed()).as_secs();

        // Don't start a new package if less than 3h15m remain.
        // This ensures we always have enough budget to finish a 3h build + upload.
        // The 20min emergency guard lives inside the loop body (mid-build / mid-upload).
        if remaining < 11700 {
            println!(">>> Less than 3:15h remaining. Not starting new packages. Finishing action.");
            break;
        }

        let pkg = packages[idx].clone();
        let is_curated = metadata[&pkg].is_curated;
        let tag = if is_curated { "[CURATED]" } else { "[AUTO]" };
        println!("\n>>> [ {}/{} ] {} Building: {}", idx + 1, packages.len(), tag, pkg);

        let local_repo = "local_repo";
        let _ = fs::remove_dir_all("result");
        let _ = fs::remove_dir_all(local_repo);

        if single_pkg.is_none() {
            if packages_since_gc >= 5 {
                println!(">>> 5 packages built! Running Nix garbage collection...");
                let _ = Command::new("timeout").args(["1200", "nix-store", "--gc"]).status();
                packages_since_gc = 0;
            } else {
                packages_since_gc += 1;
            }
        }

        println!(">>> Initializing clean local OSTree repo for this package...");
        let _ = fs::create_dir_all(local_repo);
        let _ = Command::new("ostree").args(["init", "--mode=archive-z2", &format!("--repo={}", local_repo)]).status();
        let _ = Command::new("ostree").args(["config", "--repo", local_repo, "set", "core.min-free-space-percent", "0"]).status();

        let pkg_start = Instant::now();

        // Build timeout is the lesser of:
        //   • 3h per-package hard cap
        //   • time left before the 20min pre-limit safety margin
        let run_nix = |target: &str| {
            let left_3h = limit_3h.saturating_sub(pkg_start.elapsed().as_secs());
            let left_6h = max_duration.saturating_sub(Duration::from_secs(1200)).saturating_sub(start_time.elapsed()).as_secs();
            let to = left_3h.min(left_6h).to_string();

            Command::new("timeout")
                .args([&to, "nix", "build", "--impure", "-L", target])
                .env("NIXPKGS_ALLOW_UNFREE", "1")
                .env("NIXPKGS_ALLOW_BROKEN", "1")
                .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
                .status()
        };

        let mut final_status = run_nix(&format!(".#{}", pkg));

        // Emergency check: did the build eat into our 20min safety margin?
        if max_duration.saturating_sub(start_time.elapsed()).as_secs() <= 1200 {
            println!(">>> 20min before 6h limit after first build. Stopping without saving state.");
            break;
        }

        // Only attempt the -fixed fallback if the build failed quickly (not a 3h timeout).
        if !final_status.as_ref().map_or(false, |s| s.success()) && pkg_start.elapsed().as_secs() < limit_3h - 10 {
            println!(">>> Standard build failed. Attempting -fixed icon fallback...");
            final_status = run_nix(&format!(".#{}-fixed", pkg));
        }

        // Emergency check again after the fallback build.
        if max_duration.saturating_sub(start_time.elapsed()).as_secs() <= 1200 {
            println!(">>> 20min before 6h limit after fallback build. Stopping without saving state.");
            break;
        }

        let app_id = &metadata[&pkg].app_id;
        let mut is_failure = false;
        let mut log_text = String::new();

        if final_status.as_ref().map_or(false, |s| s.success()) {
            println!(">>> Build succeeded. Importing into local repo...");
            let _ = Command::new("bash")
                .arg("-c")
                .arg(format!("flatpak build-import-bundle {} result/*.flatpak", local_repo))
                .status();

            if !app_id.is_empty() {
                println!(">>> Testing application launch on virtual display for {}...", app_id);
                let _ = Command::new("flatpak").args(["--user", "remote-add", "--no-gpg-verify", "--if-not-exists", "test_repo", local_repo]).status();
                // Timeout on install: downloading Flathub runtimes (org.gnome.Platform,
                // org.kde.Platform, etc.) can be several GB. 10 minutes is generous;
                // without this the runner hangs for hours on a slow Flathub connection.
                let _ = Command::new("timeout")
                    .args(["1200", "flatpak", "--user", "install", "--noninteractive", "-y", "test_repo", app_id])
                    .status();

                // Outer timeout on xvfb-run: the inner `timeout 10 flatpak run` only
                // covers the app process, not Xvfb startup itself. If Xvfb hangs on
                // launch (no free display, GPU init failure, etc.) the whole command
                // blocks. 60s = 10s app test + generous buffer for Xvfb startup.
                let test_output = Command::new("timeout")
                    .args([
                        "60",
                        "xvfb-run", "-a", "-s", "-screen 0 1024x768x24 +extension GLX",
                        "timeout", "10", "flatpak", "run",
                        "--env=LIBGL_ALWAYS_SOFTWARE=1", "--env=GALLIUM_DRIVER=llvmpipe", app_id
                    ]).output();

                let _ = Command::new("flatpak").args(["--user", "uninstall", "--noninteractive", "-y", app_id]).status();

                match test_output {
                    Ok(out) => {
                        let code = out.status.code().unwrap_or(0);
                        // 0 = clean exit, 124 = timed out, 137/143 = terminated — all acceptable
                        let p = code == 0 || code == 124 || code == 137 || code == 143;
                        if !p {
                            is_failure = true;
                            log_text = String::from_utf8_lossy(&out.stdout).to_string();
                            log_text.push_str("\n--- STDERR ---\n");
                            log_text.push_str(&String::from_utf8_lossy(&out.stderr));
                        }
                    },
                    Err(e) => {
                        is_failure = true;
                        log_text = format!("Failed to execute xvfb-run: {}", e);
                    }
                }
            }

            if !is_failure {
                println!(">>> Test passed! App successfully stayed alive.");

                // If less than 1h remains, skip the upload entirely.
                // Do NOT save state so this package is retried next run.
                if max_duration.saturating_sub(start_time.elapsed()).as_secs() < 3600 {
                    println!(">>> Less than 1h remaining. Skipping upload; will retry package next run.");
                    break;
                }

                println!(">>> Uploading new objects to OneDrive...");
                // Cap the upload at (time_left − 20min) so we never run into GitHub's hard kill.
                let left_6h = max_duration.saturating_sub(Duration::from_secs(1200)).saturating_sub(start_time.elapsed()).as_secs();
                let objects_status = Command::new("timeout")
                    .args([
                        &left_6h.to_string(),
                        "rclone", "copy",
                        &format!("{}/objects", local_repo), &format!("{}/objects", opts.remote),
                        "--transfers", "4", "--checkers", "8", "--tpslimit", "5",
                        "--fast-list", "--size-only",
                        "--retries", "5", "--retries-sleep", "15s",
                    ]).status();

                // Emergency check after objects upload.
                if max_duration.saturating_sub(start_time.elapsed()).as_secs() <= 1200 {
                    println!(">>> 20min before 6h limit during objects upload. Stopping without saving state.");
                    break;
                }

                if objects_status.map_or(false, |s| s.success()) {
                    let left_6h = max_duration.saturating_sub(Duration::from_secs(1200)).saturating_sub(start_time.elapsed()).as_secs();
                    let _ = Command::new("timeout")
                        .args([
                            &left_6h.to_string(),
                            "rclone", "copy", local_repo, &opts.remote,
                            "--transfers", "4", "--checkers", "8", "--tpslimit", "5",
                            "--fast-list", "--size-only",
                            "--exclude", "/objects/**",
                            "--exclude", "summary", "--exclude", "summary.sig",
                        ]).status();
                } else {
                    println!(">>> Warning: Failed to upload objects. Skipping refs upload to prevent remote corruption.");
                }

                // Emergency check after refs upload.
                if max_duration.saturating_sub(start_time.elapsed()).as_secs() <= 1200 {
                    println!(">>> 20min before 6h limit during refs upload. Stopping without saving state.");
                    break;
                }
            }
        } else if pkg_start.elapsed().as_secs() >= limit_3h - 30 {
            // Build was killed by the 3h timeout — record it as a hard failure.
            is_failure = true;
            log_text = "Build exceeded 3 hour limit and was stopped.".to_string();
        }
        // Builds that fail quickly (not a timeout) are silently skipped — we just
        // advance the state bookmark and move on.

        if is_failure {
            println!("!!! FAILED: {} crashed or timed out. Skipping upload.", app_id);

            let arch = arch_folder(&opts.system);
            let runner_failed_file = format!("failed_apps/{}/runner{}.txt", arch, opts.runner_id);
            let remote_failed_file = format!("{}/failed_apps/{}/runner{}.txt", opts.remote, arch, opts.runner_id);

            // 1. Download this runner's existing shard from OneDrive (accumulates across days).
            let left_6h = max_duration.saturating_sub(Duration::from_secs(1200)).saturating_sub(start_time.elapsed()).as_secs();
            let _ = Command::new("timeout").args([&left_6h.to_string(), "rclone", "copyto", &remote_failed_file, &runner_failed_file, "--retries", "3"]).status();

            // 2. Append to local shard.
            let _ = fs::create_dir_all(format!("failed_apps/{}", arch));
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&runner_failed_file) {
                let _ = writeln!(f, "{} ({})", pkg, app_id);
            }

            // 3. Upload updated shard back to OneDrive.
            let left_6h = max_duration.saturating_sub(Duration::from_secs(1200)).saturating_sub(start_time.elapsed()).as_secs();
            let _ = Command::new("timeout").args([&left_6h.to_string(), "rclone", "copyto", &runner_failed_file, &remote_failed_file, "--retries", "3"]).status();

            // 4. Save detailed crash/timeout log locally and upload it.
            let _ = fs::create_dir_all("failed_logs");
            let log_filename = format!("failed_logs/{}_{}.log", app_id, opts.system);
            let _ = fs::write(&log_filename, &log_text);
            let left_6h = max_duration.saturating_sub(Duration::from_secs(1200)).saturating_sub(start_time.elapsed()).as_secs();
            let _ = Command::new("timeout").args([&left_6h.to_string(), "rclone", "copyto", &log_filename, &format!("{}/failed_apps/{}_{}.log", opts.remote, app_id, opts.system)]).status();

            if max_duration.saturating_sub(start_time.elapsed()).as_secs() <= 1200 {
                println!(">>> 20min before 6h limit during failed log upload. Stopping without saving state.");
                break;
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
            continue;
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