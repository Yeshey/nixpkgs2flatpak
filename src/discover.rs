use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process::Command;

use crate::desktop_parser;
use crate::runtime_detector;
use crate::types::PackageInfo;

pub struct DiscoverOptions {
    pub output_path: String,
    /// Path to a custom nix-index database directory.
    /// Defaults to whatever nix-locate finds on its own (~/.cache/nix-index).
    pub database: Option<String>,
}

pub fn run(opts: DiscoverOptions) -> Result<()> {
    eprintln!("Querying nix-index for packages with .desktop files…");

    let packages = locate_desktop_packages(&opts.database)?;
    eprintln!("Discovered {} packages.", packages.len());

    let json = serde_json::to_string_pretty(&packages)?;
    std::fs::write(&opts.output_path, &json)
        .with_context(|| format!("Cannot write {}", opts.output_path))?;

    eprintln!("Written to {}.", opts.output_path);
    Ok(())
}

fn locate_desktop_packages(
    database: &Option<String>,
) -> Result<HashMap<String, PackageInfo>> {
    let mut cmd = Command::new("nix-locate");
    if let Some(db) = database {
        cmd.args(["--database", db]);
    }
    cmd.args([
        "--regex",
        "--at-root",
        "/share/applications/[^/]+\\.desktop$",
    ]);

    let raw = cmd
        .output()
        .context("Failed to run `nix-locate`. Is nix-index installed?")?;

    if !raw.status.success() {
        anyhow::bail!(
            "nix-locate failed:\n{}",
            String::from_utf8_lossy(&raw.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&raw.stdout);
    let mut packages: HashMap<String, PackageInfo> = HashMap::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let (attr_with_output, store_path) = match parse_line(line) {
            Some(pair) => pair,
            None => {
                eprintln!("  [skip] unparseable line: {}", line);
                continue;
            }
        };

        if ends_with_secondary_output(&attr_with_output) {
            continue;
        }

        let attr_path = strip_output_suffix(&attr_with_output).to_string();
        let pname = attr_path
            .rsplit('.')
            .next()
            .unwrap_or(&attr_path)
            .to_string();

        if packages.contains_key(&pname) {
            continue;
        }

        let desktop_filename = match store_path.rsplit('/').next() {
            Some(f) if f.ends_with(".desktop") => f.to_string(),
            _ => continue,
        };

        // ── KEY CHANGE ────────────────────────────────────────────────────────
        // Use `nix store cat` instead of reading from the local filesystem.
        // This fetches the file from the binary cache if it isn't locally present,
        // so we never need the package to be realised on this machine.
        let content = match nix_store_cat(&store_path) {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => {
                eprintln!("  [skip] empty .desktop: {}", store_path);
                continue;
            }
            Err(e) => {
                eprintln!("  [skip] nix store cat failed for {}: {}", store_path, e);
                continue;
            }
        };
        // ─────────────────────────────────────────────────────────────────────

        let fields = desktop_parser::parse(&content);

        let entry_type = fields
            .get("Type")
            .map(|s| s.as_str())
            .unwrap_or("Application");
        if entry_type != "Application" {
            continue;
        }
        if fields.get("NoDisplay").map(|s| s == "true").unwrap_or(false)
            || fields.get("Hidden").map(|s| s == "true").unwrap_or(false)
        {
            continue;
        }

        let app_id = desktop_parser::extract_app_id(&desktop_filename, &fields);
        let categories = desktop_parser::parse_categories(
            fields.get("Categories").map(|s| s.as_str()).unwrap_or(""),
        );
        let runtime_hint = runtime_detector::detect(&attr_path, &categories, &fields);

        packages.insert(
            pname.clone(),
            PackageInfo {
                attr_path,
                pname,
                app_id,
                desktop_file: desktop_filename,
                runtime_hint: runtime_hint.to_string(),
                categories,
            },
        );
    }

    Ok(packages)
}

/// Fetch a single file from the Nix store (or binary cache) without realising
/// the whole derivation. Equivalent to `nix store cat <path>`.
fn nix_store_cat(store_path: &str) -> Result<String> {
    let out = Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command",
            "store",
            "cat",
            store_path,
        ])
        .output()
        .context("Failed to invoke `nix store cat`")?;

    if !out.status.success() {
        anyhow::bail!(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn parse_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let attr = parts.next()?.to_string();
    let _size = parts.next()?;
    let _kind = parts.next()?;
    let path = parts.next()?.to_string();
    if path.starts_with('/') {
        Some((attr, path))
    } else {
        None
    }
}

fn ends_with_secondary_output(attr: &str) -> bool {
    [".dev", ".lib", ".doc", ".man", ".debug", ".info", ".static"]
        .iter()
        .any(|s| attr.ends_with(s))
}

fn strip_output_suffix(attr: &str) -> &str {
    [
        ".out", ".dev", ".lib", ".doc", ".man", ".debug", ".info", ".static",
    ]
    .iter()
    .find_map(|s| attr.strip_suffix(s))
    .unwrap_or(attr)
}