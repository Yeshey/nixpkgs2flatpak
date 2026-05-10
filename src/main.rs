use clap::{Parser, Subcommand};
use std::collections::{BTreeMap, HashMap};
use std::cmp::Reverse;

mod desktop_parser;
mod discover;
mod runtime_detector;
mod types;
mod ci_builder;
mod discover_unfree;

#[derive(Parser)]
#[command(name = "scanner")]
#[command(about = "nixpkgs2flatpak — discover nixpkgs packages and manage the Flatpak repo")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Query nix-index and write discovered.json
    Discover {
        #[arg(short, long, default_value = "discovered.json")]
        output: String,
        /// Path to a nix-index database directory built with `nix-index`.
        /// Use this to point at a database that includes unfree packages.
        #[arg(long)]
        database: Option<String>,
    },
    DiscoverUnfree {
        #[arg(short, long, default_value = "discovered.json")]
        output: String,
    },
    /// Print a summary of discovered.json
    Stats {
        #[arg(short, long, default_value = "discovered.json")]
        input: String,
    },
    /// Continuous Integration Endless Loop Builder
    BuildCi {
        #[arg(long)]
        system: String,
        /// rclone remote path, e.g. "OneDriveISCTE:nixpkgs2flatpak".
        /// State files (github_runners_state/state-<system>-runner<N>.json) are
        /// read from and written to this remote.
        #[arg(long)]
        remote: String,
        /// Which of the 2 parallel runners this process is (1 or 2).
        /// Runner 1 works forward from the start of the sorted list (a…);
        /// runner 2 works backward from the end (…z).
        /// Defaults to 1 so the command works unchanged in local/one-shot use.
        #[arg(long, default_value = "1")]
        runner_id: u8,
        /// How many minutes each runner should build before stopping gracefully.
        /// Default is 320 (5h 20m) to safely fit within GitHub's 6h job limit.
        /// Use a smaller value (e.g. 60) for quick test runs.
        #[arg(long, default_value = "320")]
        max_minutes: f64,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Discover { output, database } => {
            discover::run(discover::DiscoverOptions {
                output_path: output,
                database,
            })?
        }
        Commands::DiscoverUnfree { output } => {
            discover_unfree::run(discover_unfree::DiscoverUnfreeOptions {
                output_path: output,
            })?
        }
        Commands::Stats { input } => stats(&input)?,
        Commands::BuildCi { system, remote, runner_id, max_minutes } => {
            ci_builder::run(ci_builder::BuildCiOptions { system, remote, runner_id, max_minutes })?
        }
    }
    Ok(())
}

fn stats(path: &str) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let packages: BTreeMap<String, types::PackageInfo> = serde_json::from_str(&content)?;
    println!("Total packages: {}", packages.len());

    let mut by_runtime: HashMap<&str, usize> = HashMap::new();
    for p in packages.values() {
        *by_runtime.entry(p.runtime_hint.as_str()).or_insert(0) += 1;
    }

    let mut counts: Vec<_> = by_runtime.iter().collect();
    counts.sort_by_key(|(_, n)| Reverse(**n));
    for (runtime, count) in counts {
        println!("  {:45} {}", runtime, count);
    }
    Ok(())
}