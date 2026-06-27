use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use modelsentry_common::models::BaselineSnapshot;
use tabled::{Table, Tabled};

use crate::commands::client;

// ---------------------------------------------------------------------------
// CLI arg types
// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct BaselineArgs {
    #[command(subcommand)]
    pub action: BaselineAction,
}

#[derive(Subcommand)]
pub enum BaselineAction {
    /// Show the latest baseline for a probe
    Show {
        /// Probe ID
        probe_id: String,
    },
    /// Capture a new baseline for a probe (runs the probe now)
    Capture {
        /// Probe ID
        probe_id: String,
    },
    /// List all baselines for a probe
    List {
        /// Probe ID
        probe_id: String,
    },
}

// ---------------------------------------------------------------------------
// Table row type
// ---------------------------------------------------------------------------

#[derive(Tabled)]
struct BaselineRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Captured At")]
    captured_at: String,
    #[tabled(rename = "Prompts")]
    prompts: usize,
    #[tabled(rename = "Runs")]
    runs: usize,
    #[tabled(rename = "Run ID")]
    run_id: String,
}

fn to_row(b: &BaselineSnapshot) -> BaselineRow {
    BaselineRow {
        id: b.id.to_string(),
        captured_at: b.captured_at.to_rfc3339(),
        prompts: b.prompt_clouds.len(),
        runs: b.n_runs,
        run_id: b.run_id.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(args: BaselineArgs, api_url: &str) -> Result<()> {
    let client = client()?;

    match args.action {
        BaselineAction::Show { probe_id } => {
            let baseline: BaselineSnapshot = client
                .get(format!("{api_url}/api/probes/{probe_id}/baselines/latest"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("GET /api/probes/{probe_id}/baselines/latest failed"))?
                .json()
                .await?;

            println!("Baseline ID  : {}", baseline.id);
            println!("Probe ID     : {}", baseline.probe_id);
            println!("Captured     : {}", baseline.captured_at);
            println!("Run ID       : {}", baseline.run_id);
            println!("Schema ver   : {}", baseline.schema_version);
            println!("Embed model  : {}", baseline.embedding_model);
            println!("Prompts      : {}", baseline.prompt_clouds.len());
            println!("Runs folded  : {}", baseline.n_runs);
            println!("Embedding dim: {}", baseline.embedding_dim());
        }

        BaselineAction::Capture { probe_id } => {
            println!("Capturing baseline for probe {probe_id}…");
            let baseline: BaselineSnapshot = client
                .post(format!("{api_url}/api/probes/{probe_id}/baselines"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("POST /api/probes/{probe_id}/baselines failed"))?
                .json()
                .await?;

            println!("Captured baseline: {}", baseline.id);
            println!(
                "  {} prompt cloud(s) aggregated from {} run(s)",
                baseline.prompt_clouds.len(),
                baseline.n_runs
            );
        }

        BaselineAction::List { probe_id } => {
            let baselines: Vec<BaselineSnapshot> = client
                .get(format!("{api_url}/api/probes/{probe_id}/baselines"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("GET /api/probes/{probe_id}/baselines failed"))?
                .json()
                .await?;

            if baselines.is_empty() {
                println!("No baselines for probe {probe_id}.");
                return Ok(());
            }

            let rows: Vec<BaselineRow> = baselines.iter().map(to_row).collect();
            println!("{}", Table::new(rows));
        }
    }

    Ok(())
}
