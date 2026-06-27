//! `modelsentry-daemon` — HTTP server entry point.
//!
//! Usage:
//!   modelsentry-daemon [--config <path>]
//!
//! The default config path is `config/default.toml` relative to the working
//! directory.  Set `RUST_LOG` to control log verbosity (default: `info`).

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use modelsentry_common::config::AppConfig;
use modelsentry_core::{
    alert::AlertEngine,
    drift::{assessment::AssessmentConfig, calculator::DriftCalculator},
};
use modelsentry_daemon::{
    constants::alert::HTTP_TIMEOUT_SECS,
    provider_factory::VaultProviderResolver,
    scheduler::Scheduler,
    server::{self, AppState},
    vault::Vault,
};
use modelsentry_store::AppStore;
use secrecy::SecretString;
use tracing_subscriber::{EnvFilter, fmt};

/// `ModelSentry` daemon — LLM drift detection server.
#[derive(Parser)]
#[command(name = "modelsentry-daemon", about = "ModelSentry daemon")]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(
        long,
        default_value = "config/default.toml",
        env = "MODELSENTRY_CONFIG"
    )]
    config: PathBuf,

    /// Vault passphrase (overrides interactive prompt).
    ///
    /// In production, prefer injecting via the `MODELSENTRY_VAULT_PASSPHRASE`
    /// environment variable rather than a CLI argument.
    #[arg(long, env = "MODELSENTRY_VAULT_PASSPHRASE", hide_env_values = true)]
    vault_passphrase: Option<String>,
}

#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Logging ────────────────────────────────────────────────────────────
    fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    // ── Config ─────────────────────────────────────────────────────────────
    let cli = Cli::parse();
    let config = AppConfig::load(&cli.config)?;
    config.validate()?;
    tracing::info!(config = %cli.config.display(), "configuration loaded");
    for warning in config.security_warnings() {
        tracing::warn!("SECURITY: {warning}");
    }

    // ── Data directories ───────────────────────────────────────────────────
    if let Some(parent) = config.database.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = config.vault.path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // ── Store ──────────────────────────────────────────────────────────────
    let store = Arc::new(AppStore::open(&config.database.path)?);
    tracing::info!(path = %config.database.path.display(), "store opened");

    // ── Vault ──────────────────────────────────────────────────────────────
    // Refuse to open an existing vault without a passphrase.
    let passphrase: SecretString = match cli.vault_passphrase {
        Some(s) => SecretString::new(s.into()),
        None if config.vault.path.exists() => {
            anyhow::bail!(
                "Vault file exists at {} but MODELSENTRY_VAULT_PASSPHRASE is not set. \
                 Set the environment variable or pass --vault-passphrase.",
                config.vault.path.display()
            );
        }
        None => {
            tracing::warn!("no vault passphrase set — new vault will use an empty passphrase");
            SecretString::new(String::new().into())
        }
    };

    let vault = if config.vault.path.exists() {
        Vault::open(&config.vault.path, passphrase)?
    } else {
        tracing::info!(
            path = %config.vault.path.display(),
            "vault not found — creating empty vault"
        );
        Vault::create(&config.vault.path, passphrase)?
    };
    let vault = Arc::new(vault);

    // ── Core components (shared via Arc) ───────────────────────────────────
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()?;
    let calculator = Arc::new(DriftCalculator::new(AssessmentConfig {
        target_fpr: config.alerts.target_fpr,
        n_permutations: config.alerts.permutations,
        ..AssessmentConfig::default()
    }));
    let alert_engine = Arc::new(
        AlertEngine::new(http_client)
            .with_allow_private_targets(config.alerts.allow_private_webhook_targets),
    );

    // ── Provider resolution ──────────────────────────────────────────────
    // Providers are constructed per run from each probe's `ProviderSpec`: the
    // secret comes from the vault, the infrastructure from config. There is no
    // registry to populate at startup, and a key added later (via the vault
    // API) takes effect on the next run with no restart.
    let config = Arc::new(config);
    let resolver = Arc::new(VaultProviderResolver::new(
        Arc::clone(&vault),
        Arc::clone(&config),
    ));

    // ── Scheduler (shares same Arc'd calculator + alert engine) ────────────
    tracing::info!(
        target_fpr = config.alerts.target_fpr,
        "drift alerting calibrated to target false-positive rate",
    );
    let scheduler = Scheduler::new(
        Arc::clone(&store),
        resolver,
        Arc::clone(&calculator),
        Arc::clone(&alert_engine),
    );
    let _scheduler_handle = scheduler.start();
    tracing::info!("scheduler started");

    // ── HTTP server ─────────────────────────────────────────────────────────
    let state = AppState {
        store,
        vault,
        calculator,
        alert_engine,
        config: Arc::clone(&config),
    };

    tracing::info!(
        host = %config.server.host,
        port = config.server.port,
        "starting HTTP server"
    );
    server::run(&config, state).await?;

    Ok(())
}
