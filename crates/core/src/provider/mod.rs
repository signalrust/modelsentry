//! Provider trait and type aliases for LLM adapters.
//!
//! Every LLM backend (Anthropic, `OpenAI`, Ollama, …) implements [`LlmProvider`].
//! Use [`DynProvider`] as the concrete type wherever a provider is stored or passed.

use std::sync::Arc;

use async_trait::async_trait;
use modelsentry_common::error::Result;

use crate::drift::Embedding;

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
    ///   does not support embeddings or the request cannot be sent.
    /// - [`modelsentry_common::error::ModelSentryError::ProviderHttp`] on a non-200
    ///   HTTP response.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>>;

    /// Complete a single prompt. Returns the raw model output string.
    ///
    /// # Errors
    ///
    /// - [`modelsentry_common::error::ModelSentryError::Provider`] on network or
    ///   parsing failure.
    /// - [`modelsentry_common::error::ModelSentryError::ProviderHttp`] on a non-200
    ///   HTTP response.
    async fn complete(&self, prompt: &str) -> Result<String>;

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

pub mod anthropic;
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
            async fn complete(&self, prompt: &str) -> Result<String>;
            fn provider_name(&self) -> &'static str;
            fn embedding_dim(&self) -> usize;
        }
    }
}
