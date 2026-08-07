//! Provider trait and type aliases for LLM adapters.
//!
//! Every LLM backend (Anthropic, `OpenAI`, Ollama, …) implements [`LlmProvider`].
//! Use [`DynProvider`] as the concrete type wherever a provider is stored or passed.

use std::sync::Arc;

use async_trait::async_trait;
use modelsentry_common::error::{ModelSentryError, Result};

use crate::drift::Embedding;

/// A single model completion plus the provider-reported model identity.
///
/// `model_version` carries the provider's resolved `model` field for the
/// response (OpenAI/Azure/Ollama `model`, Anthropic `model`). It is the signal
/// used to detect a silent model-version change — the worst exchangeability
/// breaker for the conformal drift test, since it mixes a different model's
/// distribution into a baseline. `None` when the provider reports no version (or
/// reports an empty one).
#[derive(Debug, Clone)]
pub struct Completion {
    /// The model's output text.
    pub text: String,
    /// Provider-reported model version/identity for this response.
    pub model_version: Option<String>,
}

/// Trait implemented by every LLM provider adapter.
///
/// Adapters are `Send + Sync + 'static` so they can be shared across async tasks
/// without wrapping in an additional `Mutex`.
#[async_trait]
pub trait LlmProvider: Send + Sync + 'static {
    /// Embed a batch of texts. Returns one vector per input, all with identical
    /// dimension.
    ///
    /// # Errors
    ///
    /// - [`modelsentry_common::error::ModelSentryError::Provider`] if the provider
    ///   does not support embeddings.
    /// - [`modelsentry_common::error::ModelSentryError::ProviderTransport`] if the
    ///   request cannot be sent.
    /// - [`modelsentry_common::error::ModelSentryError::ProviderDecode`] if the
    ///   response cannot be decoded.
    /// - [`modelsentry_common::error::ModelSentryError::ProviderHttp`] on a non-200
    ///   HTTP response.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>>;

    /// Complete a single prompt. Returns the model output together with the
    /// provider-reported model version (see [`Completion`]).
    ///
    /// # Errors
    ///
    /// - [`modelsentry_common::error::ModelSentryError::ProviderTransport`] on a
    ///   network/transport failure.
    /// - [`modelsentry_common::error::ModelSentryError::ProviderDecode`] if the
    ///   response cannot be decoded.
    /// - [`modelsentry_common::error::ModelSentryError::Provider`] on a semantic
    ///   provider failure (e.g. an empty/blocked completion).
    /// - [`modelsentry_common::error::ModelSentryError::ProviderHttp`] on a non-200
    ///   HTTP response.
    async fn complete(&self, prompt: &str) -> Result<Completion>;

    /// Human-readable provider name for logging and error messages.
    fn provider_name(&self) -> &'static str;

    /// The embedding dimension this provider returns.
    ///
    /// Returns `0` when the provider does not support embeddings, allowing
    /// callers to guard against embedding-based drift metrics at runtime.
    fn embedding_dim(&self) -> usize;
}

/// A boxed, type-erased provider. Use this as the concrete type in `ProbeRunner`
/// and anywhere you need to store a provider without knowing its concrete type.
pub type DynProvider = Arc<dyn LlmProvider>;

/// A fresh per-request cache-busting nonce.
///
/// Conformal/permutation validity assumes baseline/run exchangeability. A
/// provider- or proxy-side cache that returns byte-identical completions for a
/// repeated request would collapse a prompt's baseline cloud to a near-constant
/// (faking determinism), silently breaking that assumption. Each completion
/// request carries this nonce as **non-semantic** metadata (`user` /
/// `metadata.user_id`) so an exact-request cache cannot serve a stale identical
/// answer. The model never sees it, so the probe's meaning is unchanged.
#[must_use]
pub(crate) fn cache_bust_nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Reject a chat-completions `finish_reason` that marks a completion as not a
/// usable, representative output: `"length"` (truncated at the token cap) or
/// `"content_filter"` (blocked). A truncated/filtered completion follows a
/// different distribution than a complete one, so folding it into a baseline or
/// run cloud would corrupt the drift signal — surface it as an error instead,
/// mirroring the Anthropic refusal guard. `"stop"`/`None` are usable.
///
/// Shared by the chat-completions adapters that report this field.
///
/// # Errors
///
/// [`ModelSentryError::Provider`] when `reason` is `"length"` or
/// `"content_filter"`.
pub(crate) fn reject_unusable_finish_reason(reason: Option<&str>) -> Result<()> {
    match reason {
        Some("length") => Err(ModelSentryError::Provider {
            message: "completion truncated (finish_reason: length) — raise \
                      [providers] max_tokens or shorten the prompt"
                .into(),
        }),
        Some("content_filter") => Err(ModelSentryError::Provider {
            message: "completion blocked by the provider content filter \
                      (finish_reason: content_filter)"
                .into(),
        }),
        _ => Ok(()),
    }
}

pub mod anthropic;
pub mod azure;
pub mod ollama;
pub mod openai;

#[cfg(test)]
pub mod mock {
    use async_trait::async_trait;
    use mockall::mock;
    use modelsentry_common::error::Result;

    use super::LlmProvider;
    use crate::drift::Embedding;

    mock! {
        pub LlmProvider {}

        #[async_trait]
        impl LlmProvider for LlmProvider {
            async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>>;
            async fn complete(&self, prompt: &str) -> Result<super::Completion>;
            fn provider_name(&self) -> &'static str;
            fn embedding_dim(&self) -> usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::cache_bust_nonce;

    #[test]
    fn cache_bust_nonce_is_nonempty_and_unique_per_call() {
        let a = cache_bust_nonce();
        let b = cache_bust_nonce();
        assert!(!a.is_empty());
        assert_ne!(a, b, "each request must get a fresh cache-busting nonce");
    }
}
