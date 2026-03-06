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
use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
use modelsentry_daemon::{
    scheduler::{ProviderRegistry, Scheduler},
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
    let passphrase: SecretString = cli.vault_passphrase.map_or_else(
        || SecretString::new(String::new().into()),
        |s| SecretString::new(s.into()),
    );

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

    // ── Core components ────────────────────────────────────────────────────
    let calculator = Arc::new(DriftCalculator::new(
        config.alerts.drift_threshold_kl,
        config.alerts.drift_threshold_cos,
    )?);
    let alert_engine = Arc::new(AlertEngine::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?,
    ));
    let providers: ProviderRegistry = ProviderRegistry::new();

    // ── Scheduler ──────────────────────────────────────────────────────────
    let scheduler = Scheduler::new(
        Arc::clone(&store),
        providers.clone(),
        DriftCalculator::new(
            config.alerts.drift_threshold_kl,
            config.alerts.drift_threshold_cos,
        )?,
        AlertEngine::new(reqwest::Client::new()),
    );
    let _scheduler_handle = scheduler.start();
    tracing::info!("scheduler started");

    // ── HTTP server ─────────────────────────────────────────────────────────
    let state = AppState {
        store,
        vault,
        providers: Arc::new(providers),
        calculator,
        alert_engine,
        config: Arc::new(config.clone()),
    };

    tracing::info!(
        host = %config.server.host,
        port = config.server.port,
        "starting HTTP server"
    );
    server::run(&config, state).await?;

    Ok(())
}
