use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use modelsentry_common::models::{AlertChannel, AlertEvent, AlertRule, DriftLevel};
use tabled::{Table, Tabled};

use crate::commands::client;

// ---------------------------------------------------------------------------
// CLI arg types
// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct AlertArgs {
    #[command(subcommand)]
    pub action: AlertAction,
}

#[derive(Subcommand)]
pub enum AlertAction {
    /// List recent alert events
    Events {
        /// Maximum number of events to show
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// List alert rules for a probe
    Rules {
        /// Probe ID
        probe_id: String,
    },
    /// Acknowledge an alert event
    Ack {
        /// Alert event ID
        event_id: String,
    },
    /// Delete an alert rule
    DeleteRule {
        /// Alert rule ID
        rule_id: String,
    },
}

// ---------------------------------------------------------------------------
// Table row types
// ---------------------------------------------------------------------------

#[derive(Tabled)]
struct EventRow {
    #[tabled(rename = "ID (short)")]
    id: String,
    #[tabled(rename = "Drift Level")]
    level: String,
    #[tabled(rename = "KL Div.")]
    kl: String,
    #[tabled(rename = "Cosine")]
    cosine: String,
    #[tabled(rename = "Fired At")]
    fired_at: String,
    #[tabled(rename = "Ack?")]
    acked: &'static str,
}

#[derive(Tabled)]
struct RuleRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "KL Thresh.")]
    kl_threshold: String,
    #[tabled(rename = "Cos. Thresh.")]
    cosine_threshold: String,
    #[tabled(rename = "Channels")]
    channels: String,
    #[tabled(rename = "Active")]
    active: &'static str,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn short_id(id: &str) -> String {
    id.get(..8).unwrap_or(id).to_owned()
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

fn channel_label(ch: &AlertChannel) -> &'static str {
    match ch {
        AlertChannel::Webhook { .. } => "webhook",
        AlertChannel::Slack { .. } => "slack",
        AlertChannel::Email { .. } => "email",
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(args: AlertArgs, api_url: &str) -> Result<()> {
    let client = client()?;

    match args.action {
        AlertAction::Events { limit } => {
            let events: Vec<AlertEvent> = client
                .get(format!("{api_url}/api/alerts/events?limit={limit}"))
                .send()
                .await?
                .error_for_status()
                .context("GET /api/alerts/events failed")?
                .json()
                .await?;

            if events.is_empty() {
                println!("No alert events.");
                return Ok(());
            }

            let rows: Vec<EventRow> = events
                .iter()
                .map(|e| EventRow {
                    id: short_id(&e.id.to_string()),
                    level: drift_label(&e.drift_report.drift_level).to_owned(),
                    kl: format!("{:.4}", e.drift_report.kl_divergence),
                    cosine: format!("{:.4}", e.drift_report.cosine_distance),
                    fired_at: e.fired_at.to_rfc3339(),
                    acked: if e.acknowledged { "yes" } else { "no" },
                })
                .collect();
            println!("{}", Table::new(rows));
        }

        AlertAction::Rules { probe_id } => {
            let rules: Vec<AlertRule> = client
                .get(format!("{api_url}/api/probes/{probe_id}/alerts"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("GET /api/probes/{probe_id}/alerts failed"))?
                .json()
                .await?;

            if rules.is_empty() {
                println!("No alert rules for probe {probe_id}.");
                return Ok(());
            }

            let rows: Vec<RuleRow> = rules
                .iter()
                .map(|r| {
                    let channels = r
                        .channels
                        .iter()
                        .map(channel_label)
                        .collect::<Vec<_>>()
                        .join(", ");
                    RuleRow {
                        id: r.id.to_string(),
                        kl_threshold: format!("{:.4}", r.kl_threshold),
                        cosine_threshold: format!("{:.4}", r.cosine_threshold),
                        channels,
                        active: if r.active { "yes" } else { "no" },
                    }
                })
                .collect();
            println!("{}", Table::new(rows));
        }

        AlertAction::Ack { event_id } => {
            client
                .post(format!("{api_url}/api/alerts/events/{event_id}/ack"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("POST /api/alerts/events/{event_id}/ack failed"))?;

            println!("Acknowledged alert event {event_id}");
        }

        AlertAction::DeleteRule { rule_id } => {
            client
                .delete(format!("{api_url}/api/alerts/{rule_id}"))
                .send()
                .await?
                .error_for_status()
                .with_context(|| format!("DELETE /api/alerts/{rule_id} failed"))?;

            println!("Deleted alert rule {rule_id}");
        }
    }

    Ok(())
}
