mod ingest;
mod process;
mod tui;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ai-wiki", version, about = "AI-powered wiki builder")]
struct Cli {
    /// Path to config file
    #[arg(long, default_value = "ai-wiki.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Ingest source files into the processing queue
    Ingest {
        /// File, glob pattern, or directory to ingest
        path: String,
    },
    /// Launch the TUI to monitor queue status
    Tui,
    /// Process queued items using Claude to build wiki pages
    Process {
        /// Maximum number of items to process in this batch
        #[arg(short, long, default_value = "10")]
        batch: usize,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = if cli.config.exists() {
        ai_wiki_core::config::Config::load(&cli.config)?
    } else {
        let config = ai_wiki_core::config::Config::default();
        config.save(&cli.config)?;
        eprintln!("Created default config at {}", cli.config.display());
        config
    };
    config.validate()?;

    match cli.command {
        Commands::Ingest { path } => ingest::run(&config, &path),
        Commands::Tui => tui::run(&config),
        Commands::Process { batch } => process::run(&config, batch),
    }
}
