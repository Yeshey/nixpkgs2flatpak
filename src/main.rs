use clap::{Parser, Subcommand};
use std::collections::{BTreeMap, HashMap};
use std::cmp::Reverse;

mod desktop_parser;
mod discover;
mod runtime_detector;
mod types;
mod ci_builder;

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
        /// State files (state-<system>.json) are read from and written to this remote.
        #[arg(long)]
        remote: String,
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
        Commands::Stats { input } => stats(&input)?,
        Commands::BuildCi { system, remote } => {
            ci_builder::run(ci_builder::BuildCiOptions { system, remote })?
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