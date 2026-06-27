//! Unified provider resolution — the single place that turns a probe's
//! [`ProviderSpec`] into a live [`DynProvider`].
//!
//! There is no provider registry and no per-provider special-casing: a provider
//! is built on demand from three sources, each with one responsibility:
//!
//! - **spec** (per-probe, user-chosen): model / deployment, and any instance
//!   address such as the Ollama base URL.
//! - **config** (`[providers.*]`, deployment-wide infra): base URLs, api-version,
//!   embedding model/dim, the Azure resource endpoint, token caps.
//! - **vault** (the secret): the API key, looked up by provider type
//!   ([`ProviderSpec::provider_id`]).
//!
//! Both the scheduler and the `run-now` route call [`build_provider`], so the
//! mapping from spec → provider lives in exactly one function.

use std::sync::Arc;

use modelsentry_common::{
    config::AppConfig,
    error::{ModelSentryError, Result},
    models::ProviderSpec,
    types::ApiKey,
};
use modelsentry_core::provider::{
    DynProvider, anthropic::AnthropicProvider, azure::AzureOpenAiProvider, ollama::OllamaProvider,
    openai::OpenAiProvider,
};

use crate::vault::Vault;

/// Construct the [`DynProvider`] a probe's [`ProviderSpec`] describes, pulling
/// the secret from the vault and infrastructure defaults from `config`.
///
/// # Errors
///
/// - [`ModelSentryError::Config`] if a required API key is missing from the
///   vault, or a constructor rejects its parameters (e.g. empty Azure endpoint).
/// - [`ModelSentryError::Vault`] if the vault cannot be read.
pub fn build_provider(
    spec: &ProviderSpec,
    vault: &Vault,
    config: &AppConfig,
) -> Result<DynProvider> {
    match spec {
        ProviderSpec::OpenAi { model } => {
            let key = require_key(vault, spec.provider_id())?;
            let cfg = &config.providers.openai;
            let p = OpenAiProvider::new(key, model)?
                .with_base_url(cfg.base_url.clone())
                .with_embedding_model(&cfg.embedding_model, cfg.embedding_dim)
                .with_max_tokens(cfg.max_tokens);
            Ok(Arc::new(p) as DynProvider)
        }
        ProviderSpec::Anthropic { model } => {
            let key = require_key(vault, spec.provider_id())?;
            let cfg = &config.providers.anthropic;
            let p = AnthropicProvider::new(key, model)?
                .with_base_url(cfg.base_url.clone())
                .with_max_tokens(cfg.max_tokens);
            Ok(Arc::new(p) as DynProvider)
        }
        ProviderSpec::Ollama { model, base_url } => {
            // Ollama needs no API key — the base URL is the only instance state.
            let p = OllamaProvider::new(model.clone(), base_url.clone())?;
            Ok(Arc::new(p) as DynProvider)
        }
        ProviderSpec::Azure {
            chat_deployment,
            embedding_deployment,
        } => {
            let key = require_key(vault, spec.provider_id())?;
            let cfg = &config.providers.azure;
            // Per-probe embedding deployment wins; fall back to the config-wide
            // default; if neither is set the provider runs completions-only.
            let embedding_deployment = embedding_deployment
                .clone()
                .or_else(|| cfg.embedding_deployment.clone());
            let p =
                AzureOpenAiProvider::new(key, &cfg.endpoint, chat_deployment, &cfg.api_version)?
                    .with_embedding(embedding_deployment, cfg.embedding_dim)
                    .with_max_tokens(cfg.max_tokens);
            Ok(Arc::new(p) as DynProvider)
        }
    }
}

/// Fetch a required API key from the vault, returning an actionable
/// [`ModelSentryError::Config`] when it is absent.
fn require_key(vault: &Vault, provider_id: &str) -> Result<ApiKey> {
    vault
        .get_key(provider_id)?
        .ok_or_else(|| ModelSentryError::Config {
            message: format!(
                "no API key configured for provider '{provider_id}' — store one via \
                 PUT /api/vault/keys/{provider_id}"
            ),
        })
}

/// Resolves a probe's [`ProviderSpec`] into a live provider.
///
/// A trait so the scheduler depends on the *capability* to resolve a provider,
/// not on the vault/config concretely — which lets tests inject a stub provider
/// without a vault or network.
pub trait ProviderResolver: Send + Sync + 'static {
    /// Build the provider described by `spec`.
    ///
    /// # Errors
    ///
    /// Propagates the error from the underlying construction (missing key,
    /// invalid parameters).
    fn resolve(&self, spec: &ProviderSpec) -> Result<DynProvider>;
}

/// Production [`ProviderResolver`]: resolves via the vault (secret) and config
/// (infrastructure) using [`build_provider`].
pub struct VaultProviderResolver {
    vault: Arc<Vault>,
    config: Arc<AppConfig>,
}

impl VaultProviderResolver {
    #[must_use]
    pub fn new(vault: Arc<Vault>, config: Arc<AppConfig>) -> Self {
        Self { vault, config }
    }
}

impl ProviderResolver for VaultProviderResolver {
    fn resolve(&self, spec: &ProviderSpec) -> Result<DynProvider> {
        build_provider(spec, &self.vault, &self.config)
    }
}
