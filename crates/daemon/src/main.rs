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
    drift::calculator::DriftCalculator,
    provider::{anthropic::AnthropicProvider, ollama::OllamaProvider, openai::OpenAiProvider},
};
use modelsentry_daemon::{
    scheduler::{Scheduler, new_registry},
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
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let calculator = Arc::new(DriftCalculator::new(
        config.alerts.drift_threshold_kl,
        config.alerts.drift_threshold_cos,
    )?);
    let alert_engine = Arc::new(AlertEngine::new(http_client));

    // ── Providers — load API keys from vault ─────────────────────────────
    let provider_map = new_registry();

    // OpenAI
    match vault.get_key("openai") {
        Ok(Some(key)) => {
            match OpenAiProvider::new(key, &config.providers.openai.model) {
                Ok(p) => {
                    let p = p
                        .with_base_url(config.providers.openai.base_url.clone())
                        .with_embedding_model(
                            &config.providers.openai.embedding_model,
                            config.providers.openai.embedding_dim,
                        );
                    provider_map
                        .write()
                        .expect("provider registry poisoned")
                        .insert("openai".to_string(), Arc::new(p));
                    tracing::info!("provider registered: openai");
                }
                Err(e) => tracing::warn!("failed to initialise OpenAI provider: {e}"),
            }
        }
        Ok(None) => tracing::debug!("no 'openai' key in vault — openai provider not registered"),
        Err(e) => tracing::warn!("vault error reading 'openai' key: {e}"),
    }

    // Anthropic
    match vault.get_key("anthropic") {
        Ok(Some(key)) => {
            match AnthropicProvider::new(key, &config.providers.anthropic.model) {
                Ok(p) => {
                    let p = p.with_base_url(config.providers.anthropic.base_url.clone());
                    provider_map
                        .write()
                        .expect("provider registry poisoned")
                        .insert("anthropic".to_string(), Arc::new(p));
                    tracing::info!("provider registered: anthropic");
                }
                Err(e) => tracing::warn!("failed to initialise Anthropic provider: {e}"),
            }
        }
        Ok(None) => {
            tracing::debug!("no 'anthropic' key in vault — anthropic provider not registered");
        }
        Err(e) => tracing::warn!("vault error reading 'anthropic' key: {e}"),
    }

    // Ollama (no API key — just needs base_url stored as the 'key' value)
    // Convention: store the base URL as the vault value for provider id 'ollama'.
    // Falls back to config default if no entry exists.
    {
        let base_url = match vault.get_key("ollama") {
            Ok(Some(key)) => key.expose().to_string(),
            _ => config.providers.ollama.base_url.clone(),
        };
        let ollama_model = match vault.get_key("ollama:model") {
            Ok(Some(key)) => key.expose().to_string(),
            _ => config.providers.ollama.model.clone(),
        };
        match OllamaProvider::new(ollama_model.clone(), base_url.clone()) {
            Ok(p) => {
                provider_map
                    .write()
                    .expect("provider registry poisoned")
                    .insert(format!("ollama:{base_url}"), Arc::new(p));
                tracing::info!(base_url = %base_url, model = %ollama_model, "provider registered: ollama");
            }
            Err(e) => tracing::warn!("failed to initialise Ollama provider: {e}"),
        }
    }

    let providers = Arc::new(provider_map);
    tracing::info!(
        registered = providers
            .read()
            .expect("provider registry poisoned")
            .len(),
        "provider registry built"
    );

    // ── Scheduler (shares same Arc'd calculator + alert engine) ────────────
    let scheduler = Scheduler::new(
        Arc::clone(&store),
        Arc::clone(&providers),
        Arc::clone(&calculator),
        Arc::clone(&alert_engine),
    );
    let _scheduler_handle = scheduler.start();
    tracing::info!("scheduler started");

    // ── HTTP server ─────────────────────────────────────────────────────────
    let state = AppState {
        store,
        vault,
        providers,
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
