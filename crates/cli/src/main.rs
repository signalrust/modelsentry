mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};
use commands::{alert::AlertArgs, baseline::BaselineArgs, probe::ProbeArgs};

#[derive(Parser)]
#[command(
    name = "modelsentry",
    about = "ModelSentry CLI — manage and monitor LLM probes",
    version
)]
struct Cli {
    /// Base URL of the `ModelSentry` daemon API
    #[arg(long, default_value = "http://localhost:7740", env = "MODELSENTRY_URL")]
    api_url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage probes
    Probe(ProbeArgs),
    /// Manage baselines
    Baseline(BaselineArgs),
    /// Manage alert rules and events
    Alert(AlertArgs),
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let api_url = cli.api_url.trim_end_matches('/').to_owned();

    match cli.command {
        Commands::Probe(args) => commands::probe::handle(args, &api_url).await,
        Commands::Baseline(args) => commands::baseline::handle(args, &api_url).await,
        Commands::Alert(args) => commands::alert::handle(args, &api_url).await,
    }
}
