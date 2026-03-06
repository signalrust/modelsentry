use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use modelsentry_common::models::{DriftLevel, Probe, ProbeRun, ProviderKind};
use tabled::{Table, Tabled};

use crate::commands::client;

// ---------------------------------------------------------------------------
// CLI arg types
// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct ProbeArgs {
    #[command(subcommand)]
    pub action: ProbeAction,
}

#[derive(Subcommand)]
pub enum ProbeAction {
    /// List all configured probes
    List,
    /// Add a new probe from a TOML config file
    Add {
        /// Path to probe TOML config file
        #[arg(long)]
        config: PathBuf,
    },
    /// Delete a probe by ID
    Delete {
        /// Probe ID
        id: String,
    },
    /// Trigger an immediate probe run
    RunNow {
        /// Probe ID
        id: String,
    },
    /// Show the last drift report for a probe
    Status {
        /// Probe ID
        id: String,
    },
}

// ---------------------------------------------------------------------------
// Table row types
// ---------------------------------------------------------------------------

#[derive(Tabled)]
struct ProbeRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Provider")]
    provider: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(rename = "Schedule")]
    schedule: String,
}

#[derive(Tabled)]
struct DriftRow {
    #[tabled(rename = "Metric")]
    metric: String,
    #[tabled(rename = "Value")]
    value: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn provider_label(p: &ProviderKind) -> String {
    match p {
        ProviderKind::OpenAi => "openai".to_owned(),
        ProviderKind::Anthropic => "anthropic".to_owned(),
        ProviderKind::Ollama { base_url } => format!("ollama ({base_url})"),
        ProviderKind::AzureOpenAi { deployment, .. } => format!("azure ({deployment})"),
    }
}

fn drift_label(level: &DriftLevel) -> &'static str {
    match level {
        DriftLevel::None => "none",
        DriftLevel::Low => "low",
        DriftLevel::Medium => "medium",
        DriftLevel::High => "HIGH",
        DriftLevel::Critical => "CRITICAL",
    }
}

fn print_run_result(run: &ProbeRun) {
    println!("Run ID  : {}", run.id);
    println!("Status  : {:?}", run.status);
    println!("Started : {}", run.started_at);
    println!("Finished: {}", run.finished_at);

    if let Some(report) = &run.drift_report {
        println!();
        let rows = vec![
            DriftRow {
                metric: "Drift Level".to_owned(),
                value: drift_label(&report.drift_level).to_owned(),
            },
            DriftRow {
                metric: "KL Divergence".to_owned(),
                value: format!("{:.6}", report.kl_divergence),
            },
            DriftRow {
                metric: "Cosine Distance".to_owned(),
                value: format!("{:.6}", report.cosine_distance),
            },
            DriftRow {
                metric: "Entropy Delta".to_owned(),
                value: format!("{:.6}", report.output_entropy_delta),
            },
        ];
        println!("{}", Table::new(rows));
    } else {
        println!("(no drift report — baseline may not exist yet)");
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(args: ProbeArgs, api_url: &str) -> Result<()> {
    let client = client()?;

    match args.action {
        ProbeAction::List => {
            let probes: Vec<Probe> = client
                .get(format!("{api_url}/api/probes"))
                .send()
                .await?
                .error_for_status()
                .context("GET /api/probes failed")?
                .json()
                .await?;

            if probes.is_empty() {
                println!("No probes configured.");
                return Ok(());
            }

            // Fetch latest run per probe for drift column.
            let mut rows: Vec<ProbeRow> = Vec::with_capacity(probes.len());
            for probe in &probes {
                let schedule_label = match &probe.schedule {
                    modelsentry_common::models::ProbeSchedule::Cron { expression } => {
                        expression.clone()
                    }
                    modelsentry_common::models::ProbeSchedule::EveryMinutes { minutes } => {
                        format!("every {minutes}m")
                    }
                };
                rows.push(ProbeRow {
                    id: probe.id.to_string(),
                    name: probe.name.clone(),
                    provider: provider_label(&probe.provider),
                    model: probe.model.clone(),
                    schedule: schedule_label,
                });
            }
            println!("{}", Table::new(rows));
        }

        ProbeAction::Add { config } => {
            let toml_str = std::fs::read_to_string(&config)
                .with_context(|| format!("cannot read {}", config.display()))?;
            let body: serde_json::Value = toml::from_str(&toml_str)
                .with_context(|| format!("invalid TOML in {}", config.display()))?;

            let probe: Probe = client
                .post(format!("{api_url}/api/probes"))
                .json(&body)
                .send()
                .await?
                .error_for_status()
                .context("POST /api/probes failed")?
                .json()
                .await?;

            println!("Created probe: {}", probe.id);
            println!("  Name: {}", probe.name);
        }

        ProbeAction::Delete { id } => {
            client
                .delete(format!("{api_url}/api/probes/{id}"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("DELETE /api/probes/{id} failed"))?;

            println!("Deleted probe {id}");
        }

        ProbeAction::RunNow { id } => {
            println!("Triggering run for probe {id}…");
            let run: ProbeRun = client
                .post(format!("{api_url}/api/probes/{id}/run-now"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("POST /api/probes/{id}/run-now failed"))?
                .json()
                .await?;

            print_run_result(&run);
        }

        ProbeAction::Status { id } => {
            let runs: Vec<ProbeRun> = client
                .get(format!("{api_url}/api/probes/{id}/runs?limit=1"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("GET /api/probes/{id}/runs failed"))?
                .json()
                .await?;

            match runs.into_iter().next() {
                Some(run) => print_run_result(&run),
                None => println!("No runs found for probe {id}"),
            }
        }
    }

    Ok(())
}
