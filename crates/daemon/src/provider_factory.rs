//! Shared provider construction — single source of truth for building
//! [`DynProvider`] instances from a provider ID, API key, and config.
//!
//! Used by both the startup code (`main.rs`) and the vault upsert endpoint
//! (`routes/vault.rs`) so that provider wiring is never duplicated.

use std::sync::Arc;

use modelsentry_common::{config::AppConfig, error::Result, types::ApiKey};
use modelsentry_core::provider::{
    DynProvider, anthropic::AnthropicProvider, ollama::OllamaProvider, openai::OpenAiProvider,
};

/// Optional overrides supplied by the caller (e.g. from a vault upsert
/// request body). Fields that are `None` fall back to the corresponding
/// value in [`AppConfig`].
#[derive(Debug, Default)]
pub struct ProviderOverrides {
    /// Model name override.
    pub model: Option<String>,
    /// Base URL override (only meaningful for Ollama).
    pub base_url: Option<String>,
}

/// Attempt to construct a [`DynProvider`] from a known provider ID.
///
/// Returns `Ok(Some(provider))` on success, `Ok(None)` for unrecognised
/// provider IDs, or `Err` if construction fails.
///
/// # Errors
///
/// Returns [`ModelSentryError`](modelsentry_common::error::ModelSentryError)
/// if the provider constructor rejects the supplied key or model parameters.
pub fn build_provider(
    provider_id: &str,
    key: ApiKey,
    overrides: &ProviderOverrides,
    config: &AppConfig,
) -> Result<Option<DynProvider>> {
    match provider_id {
        "openai" => {
            let model = overrides
                .model
                .as_deref()
                .unwrap_or(&config.providers.openai.model);
            let p = OpenAiProvider::new(key, model)?
                .with_base_url(config.providers.openai.base_url.clone())
                .with_embedding_model(
                    &config.providers.openai.embedding_model,
                    config.providers.openai.embedding_dim,
                );
            Ok(Some(Arc::new(p) as DynProvider))
        }
        "anthropic" => {
            let model = overrides
                .model
                .as_deref()
                .unwrap_or(&config.providers.anthropic.model);
            let p = AnthropicProvider::new(key, model)?
                .with_base_url(config.providers.anthropic.base_url.clone());
            Ok(Some(Arc::new(p) as DynProvider))
        }
        id if id.starts_with("ollama") => {
            let key_str = key.expose().trim().to_string();
            let base_url = if key_str.is_empty() {
                overrides
                    .base_url
                    .clone()
                    .unwrap_or_else(|| config.providers.ollama.base_url.clone())
            } else {
                key_str
            };
            let model = overrides
                .model
                .as_deref()
                .unwrap_or(&config.providers.ollama.model);
            let p = OllamaProvider::new(model.to_string(), base_url)?;
            Ok(Some(Arc::new(p) as DynProvider))
        }
        _ => Ok(None),
    }
}
