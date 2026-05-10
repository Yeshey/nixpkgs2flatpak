use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::process::Command;
use serde::Deserialize;

use crate::desktop_parser;
use crate::runtime_detector;
use crate::types::PackageInfo;

pub struct DiscoverUnfreeOptions {
    /// Path to the discovered.json file to merge results into.
    pub output_path: String,
}

/// One entry returned by ci/discover-unfree.nix.
#[derive(Deserialize)]
struct NixEntry {
    pname: String,
    stem:  String,
}

pub fn run(opts: DiscoverUnfreeOptions) -> Result<()> {
    // Load whatever is already in discovered.json so we can skip known packages
    // and write back a merged result.
    let mut packages: BTreeMap<String, PackageInfo> =
        std::fs::read_to_string(&opts.output_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default();

    let before = packages.len();

    // ── Pass 1: nix eval for desktopItems ──────────────────────────────────
    // Evaluates the full nixpkgs top-level attribute set with allowUnfree=true
    // and finds every package whose `desktopItems` list is non-empty.
    // This is the pattern used by makeDesktopItem — the standard nixpkgs helper
    // for unfree GUI apps (Slack, Discord, Spotify, Steam, etc.).
    eprintln!("Pass 1: evaluating nixpkgs package set for desktopItems (allowUnfree=true)…");
    match nix_eval_pass() {
        Ok(found) => {
            let mut added = 0usize;
            for (pname, info) in found {
                if !packages.contains_key(&pname) {
                    packages.insert(pname, info);
                    added += 1;
                }
            }
            eprintln!("  → {} new packages discovered via nix eval.", added);
        }
        Err(e) => eprintln!("  ! nix eval pass failed: {:#}", e),
    }

    // ── Pass 2: grep nixpkgs/pkgs/by-name for share/applications ──────────
    // Catches packages that install a .desktop file from their source tarball
    // without using makeDesktopItem — the nix eval pass misses these because
    // they have no `desktopItems` attribute.
    //
    // We restrict to pkgs/by-name/ because the path→pname mapping is
    // unambiguous there: pkgs/by-name/{2-char prefix}/{pname}/package.nix → pname.
    // Other directory trees (pkgs/applications/, pkgs/games/, …) require
    // consulting all-packages.nix to resolve the attribute name, which is
    // significantly more complex and less reliable.
    eprintln!("Pass 2: grepping nixpkgs source for .desktop file installations…");
    match nixpkgs_source_path() {
        Ok(src) => {
            match grep_pass(&src, &packages) {
                Ok(found) => {
                    let mut added = 0usize;
                    for (pname, info) in found {
                        if !packages.contains_key(&pname) {
                            packages.insert(pname, info);
                            added += 1;
                        }
                    }
                    eprintln!("  → {} new packages discovered via grep.", added);
                }
                Err(e) => eprintln!("  ! grep pass failed: {:#}", e),
            }
        }
        Err(e) => eprintln!("  ! could not resolve nixpkgs source path: {:#}", e),
    }

    let added = packages.len().saturating_sub(before);
    eprintln!(
        "Total new packages added: {}  |  Grand total: {}",
        added,
        packages.len()
    );

    let json = serde_json::to_string_pretty(&packages)?;
    std::fs::write(&opts.output_path, &json)
        .with_context(|| format!("Cannot write {}", opts.output_path))?;
    eprintln!("Written to {}.", opts.output_path);
    Ok(())
}

// ── Pass 1: nix eval ────────────────────────────────────────────────────────

fn nix_eval_pass() -> Result<BTreeMap<String, PackageInfo>> {
    eprintln!("  (this evaluates the full nixpkgs attribute set — may take several minutes)");

    let raw = Command::new("nix")
        .args(["eval", "--json", "--impure", "-f", "ci/discover-unfree.nix"])
        .env("NIXPKGS_ALLOW_UNFREE", "1")
        .env("NIXPKGS_ALLOW_BROKEN", "1")
        .env("NIXPKGS_ALLOW_UNSUPPORTED_SYSTEM", "1")
        .output()
        .context("Failed to spawn `nix eval`")?;

    if !raw.status.success() {
        anyhow::bail!(
            "`nix eval` exited non-zero:\n{}",
            String::from_utf8_lossy(&raw.stderr)
        );
    }

    let entries: Vec<NixEntry> =
        serde_json::from_slice(&raw.stdout).context("Failed to parse nix eval JSON output")?;

    let mut out = BTreeMap::new();
    for e in entries {
        let desktop_file = format!("{}.desktop", e.stem);
        let app_id       = desktop_parser::extract_app_id(&desktop_file, &e.pname);
        let runtime      = runtime_detector::detect(&e.pname, &desktop_file);

        out.insert(
            e.pname.clone(),
            PackageInfo {
                // Top-level nixpkgs packages: attrPath == pname.
                // packages.nix's safeGetPkg resolves this via lib.getAttrFromPath.
                attr_path:    e.pname.clone(),
                pname:        e.pname.clone(),
                app_id,
                desktop_file,
                runtime_hint: runtime.to_string(),
            },
        );
    }

    Ok(out)
}

// ── Pass 2: grep ─────────────────────────────────────────────────────────────

/// Resolve the nixpkgs source tree from the flake's lockfile.
fn nixpkgs_source_path() -> Result<String> {
    let raw = Command::new("nix")
        .args([
            "eval", "--raw", "--impure",
            "--expr", &format!(
                r#"(builtins.getFlake "path:{}").inputs.nixpkgs.outPath"#,
                std::env::current_dir()
                    .context("Cannot determine current directory")?
                    .display()
            ),
        ])
        .output()
        .context("Failed to spawn `nix eval` for nixpkgs path")?;

    if !raw.status.success() {
        anyhow::bail!(
            "Could not resolve nixpkgs.outPath:\n{}",
            String::from_utf8_lossy(&raw.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&raw.stdout).trim().to_string())
}

fn grep_pass(
    nixpkgs_src: &str,
    known: &BTreeMap<String, PackageInfo>,
) -> Result<BTreeMap<String, PackageInfo>> {
    let by_name = format!("{}/pkgs/by-name", nixpkgs_src);

    // `share/applications` is the canonical XDG install path for .desktop files.
    // Any nix file in pkgs/by-name that mentions it is almost certainly
    // installing a desktop file (either via copyDesktopItems, cmake install, or
    // a manual install command). The false-positive rate is negligible: a failed
    // build attempt is the worst outcome.
    let raw = Command::new("grep")
        .args([
            "-rl",              // recursive, list matching filenames only
            "--include=*.nix",
            "share/applications",
            &by_name,
        ])
        .output()
        .context("Failed to spawn `grep`")?;

    // grep exits 1 when there are zero matches — that is not an error for us.
    let stdout = String::from_utf8_lossy(&raw.stdout);
    let mut out = BTreeMap::new();

    for path in stdout.lines() {
        let pname = match by_name_pname(path) {
            Some(p) => p,
            None    => continue,
        };

        // Skip packages already known from discovered.json or the nix eval pass.
        if known.contains_key(pname) {
            continue;
        }

        // Use the pname as the desktop stem fallback.
        // discover.rs does the same for packages found via nix-locate.
        let desktop_file = format!("{}.desktop", pname);
        let app_id       = desktop_parser::extract_app_id(&desktop_file, pname);
        let runtime      = runtime_detector::detect(pname, &desktop_file);

        out.insert(
            pname.to_string(),
            PackageInfo {
                attr_path:    pname.to_string(),
                pname:        pname.to_string(),
                app_id,
                desktop_file,
                runtime_hint: runtime.to_string(),
            },
        );
    }

    Ok(out)
}

/// Extract the package name from a pkgs/by-name path.
///
/// Layout: `.../pkgs/by-name/{2-char prefix}/{pname}/package.nix`
/// Splitting from the right gives us the filename, then the pname directory.
fn by_name_pname(path: &str) -> Option<&str> {
    if !path.contains("/by-name/") {
        return None;
    }
    let mut parts = path.rsplitn(3, '/');
    let _file  = parts.next()?;   // "package.nix"
    let pname  = parts.next()?;   // the package name directory
    let _rest  = parts.next()?;   // rest of path (must exist to confirm depth)
    Some(pname)
}
