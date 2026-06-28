//! Ollama API adapter.
//!
//! Implements [`LlmProvider`] for self-hosted Ollama (`/api/generate` and
//! `/api/embeddings`) using the Ollama HTTP API.
//!
//! **API key:** Ollama has no authentication by default; [`OllamaProvider`]
//! does not require an API key. Pass `ApiKey::new(String::new())` if a
//! credential is needed for a reverse-proxy.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use modelsentry_common::constants::defaults;
use modelsentry_common::error::{ModelSentryError, Result};

use super::LlmProvider;
use crate::drift::Embedding;

// ── Public type ───────────────────────────────────────────────────────────────

/// Ollama HTTP API adapter.
///
/// Built per run from a **shared** [`reqwest::Client`] (injected, so the
/// connection pool is reused across runs rather than rebuilt each time). The
/// per-request timeout is applied on each call.
#[derive(Debug)]
pub struct OllamaProvider {
    client: reqwest::Client,
    request_timeout: std::time::Duration,
    model: String,
    base_url: String,
    embed_dim: usize,
}

impl OllamaProvider {
    /// Create a provider over the shared `client`, targeting a local Ollama
    /// server.
    ///
    /// `base_url` should be the root of the server, e.g.
    /// `http://localhost:11434`. A 120-second per-request timeout is used
    /// because first-run generation can be slow on CPU.
    ///
    /// # Errors
    ///
    /// [`ModelSentryError::Config`] if `model` is empty.
    pub fn new(
        client: reqwest::Client,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self> {
        let model = model.into();
        if model.is_empty() {
            return Err(ModelSentryError::Config {
                message: "model name must not be empty".into(),
            });
        }
        Ok(Self {
            client,
            request_timeout: std::time::Duration::from_secs(defaults::ollama::TIMEOUT_SECS),
            model,
            base_url: base_url.into(),
            embed_dim: defaults::ollama::EMBEDDING_DIM,
        })
    }

    /// Override the API base URL (primarily for tests that point at a mock server).
    #[must_use]
    pub fn with_base_url(self, base_url: String) -> Self {
        Self { base_url, ..self }
    }

    /// Override the nominal embedding dimension reported by [`embedding_dim`].
    ///
    /// Ollama's true embedding width is model-dependent; set it here if you know
    /// it (query `/api/show` → `embedding_length`). It is used only for the
    /// has-embeddings capability check, not validated against responses.
    ///
    /// [`embedding_dim`]: LlmProvider::embedding_dim
    #[must_use]
    pub fn with_embedding_dim(self, embed_dim: usize) -> Self {
        Self { embed_dim, ..self }
    }
}

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embedding: Vec<f32>,
}

// ── LlmProvider impl ──────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for OllamaProvider {
    /// Embed texts one at a time via `POST /api/embeddings`.
    ///
    /// Ollama's embeddings endpoint accepts a single prompt per request, so
    /// requests are issued sequentially. Use small batches in production to
    /// avoid saturating the local server.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Provider`] on network or parse failure.
    /// - [`ModelSentryError::ProviderHttp`] on a non-200 HTTP status.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            let url = format!("{}/api/embeddings", self.base_url);
            let body = EmbedRequest {
                model: &self.model,
                prompt: text,
            };
            let response = self
                .client
                .post(&url)
                .timeout(self.request_timeout)
                .json(&body)
                .send()
                .await
                .map_err(|e| ModelSentryError::Provider {
                    message: format!("HTTP request failed: {e}"),
                })?;

            let status = response.status().as_u16();
            if status != 200 {
                let body_text = response.text().await.unwrap_or_default();
                return Err(ModelSentryError::ProviderHttp {
                    status,
                    body: body_text,
                });
            }

            let parsed: EmbedResponse =
                response
                    .json()
                    .await
                    .map_err(|e| ModelSentryError::Provider {
                        message: format!("failed to deserialize embeddings response: {e}"),
                    })?;

            results.push(Embedding::new(parsed.embedding)?);
        }
        Ok(results)
    }

    /// Generate a completion via `POST /api/generate` (non-streaming).
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Provider`] on network or parse failure.
    /// - [`ModelSentryError::ProviderHttp`] on a non-200 HTTP status.
    async fn complete(&self, prompt: &str) -> Result<String> {
        let url = format!("{}/api/generate", self.base_url);
        let body = GenerateRequest {
            model: &self.model,
            prompt,
            stream: false,
        };
        let response = self
            .client
            .post(&url)
            .timeout(self.request_timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| ModelSentryError::Provider {
                message: format!("HTTP request failed: {e}"),
            })?;

        let status = response.status().as_u16();
        if status != 200 {
            let body_text = response.text().await.unwrap_or_default();
            return Err(ModelSentryError::ProviderHttp {
                status,
                body: body_text,
            });
        }

        let parsed: GenerateResponse =
            response
                .json()
                .await
                .map_err(|e| ModelSentryError::Provider {
                    message: format!("failed to deserialize generate response: {e}"),
                })?;

        Ok(parsed.response)
    }

    fn provider_name(&self) -> &'static str {
        modelsentry_common::constants::provider::OLLAMA
    }

    /// Returns the configured nominal embedding dimension (see
    /// [`OllamaProvider::with_embedding_dim`]). Defaults to a conservative value
    /// covering popular embedding models; it gates the has-embeddings check and
    /// is not validated against actual responses, whose width is model-dependent.
    fn embedding_dim(&self) -> usize {
        self.embed_dim
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn make_provider(base_url: &str) -> OllamaProvider {
        OllamaProvider::new(reqwest::Client::new(), defaults::ollama::MODEL, base_url)
            .expect("valid provider config")
    }

    fn generate_ok(text: &str) -> serde_json::Value {
        serde_json::json!({
            "model": "llama3",
            "response": text,
            "done": true
        })
    }

    fn embed_ok(v: &[f32]) -> serde_json::Value {
        serde_json::json!({ "embedding": v })
    }

    #[tokio::test]
    async fn complete_parses_ollama_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(generate_ok("Hello!")))
            .mount(&server)
            .await;

        let result = make_provider(&server.uri())
            .complete("Say hello")
            .await
            .unwrap();
        assert_eq!(result, "Hello!");
    }

    #[tokio::test]
    async fn embed_returns_embeddings() {
        let server = MockServer::start().await;
        let vec1 = vec![0.1_f32, 0.2, 0.3];
        Mock::given(method("POST"))
            .and(path("/api/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embed_ok(&vec1.clone())))
            .mount(&server)
            .await;

        let result = make_provider(&server.uri())
            .embed(&["hello".into()])
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].as_slice(), vec1.as_slice());
    }

    #[tokio::test]
    async fn complete_returns_error_on_500() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .complete("hello")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ModelSentryError::ProviderHttp { status: 500, .. }
        ));
    }

    #[tokio::test]
    async fn embed_returns_error_on_503() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/embeddings"))
            .respond_with(ResponseTemplate::new(503).set_body_string("model not loaded"))
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .embed(&["text".into()])
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ModelSentryError::ProviderHttp { status: 503, .. }
        ));
    }

    #[test]
    fn empty_model_is_rejected() {
        let err = OllamaProvider::new(reqwest::Client::new(), "", defaults::ollama::BASE_URL)
            .unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }
}
