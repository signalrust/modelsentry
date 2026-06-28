//! `OpenAI` Chat Completions + Embeddings API adapter.
//!
//! Implements [`LlmProvider`] for `gpt-*` model family via
//! `/v1/chat/completions` and `/v1/embeddings`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use modelsentry_common::{
    error::{ModelSentryError, Result},
    types::ApiKey,
};

use modelsentry_common::constants::defaults;

use super::LlmProvider;
use crate::drift::Embedding;

// ── Public type ───────────────────────────────────────────────────────────────

/// `OpenAI` Chat Completions + Embeddings API adapter.
///
/// Built per run from a **shared** [`reqwest::Client`] (injected, so the
/// connection pool is reused across runs rather than rebuilt each time). The
/// per-request timeout is applied on each call.
#[derive(Debug)]
pub struct OpenAiProvider {
    api_key: ApiKey,
    client: reqwest::Client,
    request_timeout: std::time::Duration,
    model: String,
    embedding_model: String,
    embed_dim: usize,
    base_url: String,
    max_tokens: u32,
}

impl OpenAiProvider {
    /// Create a provider over the shared `client`, with a 30-second per-request
    /// timeout and the default embedding model (`text-embedding-3-small`, 1536
    /// dims).
    ///
    /// # Errors
    ///
    /// [`ModelSentryError::Config`] if `model` is empty.
    pub fn new(client: reqwest::Client, api_key: ApiKey, model: impl Into<String>) -> Result<Self> {
        let model = model.into();
        if model.is_empty() {
            return Err(ModelSentryError::Config {
                message: "model name must not be empty".into(),
            });
        }

        Ok(Self {
            api_key,
            client,
            request_timeout: std::time::Duration::from_secs(defaults::openai::TIMEOUT_SECS),
            model,
            embedding_model: defaults::openai::EMBEDDING_MODEL.to_string(),
            embed_dim: defaults::openai::EMBEDDING_DIM,
            base_url: defaults::openai::BASE_URL.to_string(),
            max_tokens: defaults::MAX_TOKENS,
        })
    }

    /// Override the per-request `max_tokens` (completion length cap).
    #[must_use]
    pub fn with_max_tokens(self, max_tokens: u32) -> Self {
        Self { max_tokens, ..self }
    }

    /// Override the embedding model and its output dimension.
    ///
    /// | Model | Dimension |
    /// |---|---|
    /// | `text-embedding-3-small` | 1536 |
    /// | `text-embedding-3-large` | 3072 |
    /// | `text-embedding-ada-002` | 1536 |
    #[must_use]
    pub fn with_embedding_model(self, model: impl Into<String>, dim: usize) -> Self {
        Self {
            embedding_model: model.into(),
            embed_dim: dim,
            ..self
        }
    }

    /// Override the API base URL (primarily for tests that point at a mock server).
    #[must_use]
    pub fn with_base_url(self, base_url: String) -> Self {
        Self { base_url, ..self }
    }
}

// ── Wire types (private) ──────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    // GPT-5 / reasoning models reject the legacy `max_tokens` (400) and require
    // `max_completion_tokens`; it is also accepted by older chat models.
    max_completion_tokens: u32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageContent,
}

#[derive(Deserialize)]
struct ChatMessageContent {
    content: String,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

// ── LlmProvider impl ──────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for OpenAiProvider {
    /// Embed a batch of texts using `text-embedding-3-small` (or the model set
    /// via [`with_embedding_model`]).
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Provider`] on network or parse failure.
    /// - [`ModelSentryError::ProviderHttp`] on a non-200 HTTP status.
    /// - [`ModelSentryError::DimensionMismatch`] if the API returns embeddings
    ///   of a different width than the configured `embed_dim` (i.e. the
    ///   embedding model was changed without updating `embedding_dim`).
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        let url = format!("{}/v1/embeddings", self.base_url);

        let request_body = EmbedRequest {
            model: &self.embedding_model,
            input: texts,
        };

        let response = self
            .client
            .post(&url)
            .timeout(self.request_timeout)
            .bearer_auth(self.api_key.expose())
            .json(&request_body)
            .send()
            .await
            .map_err(|e| ModelSentryError::Provider {
                message: format!("HTTP request failed: {e}"),
            })?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response.text().await.unwrap_or_default();
            return Err(ModelSentryError::ProviderHttp { status, body });
        }

        let parsed: EmbedResponse =
            response
                .json()
                .await
                .map_err(|e| ModelSentryError::Provider {
                    message: format!("failed to deserialize embeddings response: {e}"),
                })?;

        parsed
            .data
            .into_iter()
            .map(|d| {
                if d.embedding.len() != self.embed_dim {
                    return Err(ModelSentryError::DimensionMismatch {
                        expected: self.embed_dim,
                        actual: d.embedding.len(),
                    });
                }
                Embedding::new(d.embedding)
            })
            .collect()
    }

    /// Send a single-turn chat message and return the assistant reply.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Provider`] on network or parse failure.
    /// - [`ModelSentryError::ProviderHttp`] on a non-200 HTTP status (e.g.
    ///   429 rate-limit).
    async fn complete(&self, prompt: &str) -> Result<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        let request_body = ChatRequest {
            model: &self.model,
            max_completion_tokens: self.max_tokens,
            messages: vec![ChatMessage {
                role: "user",
                content: prompt,
            }],
        };

        let response = self
            .client
            .post(&url)
            .timeout(self.request_timeout)
            .bearer_auth(self.api_key.expose())
            .json(&request_body)
            .send()
            .await
            .map_err(|e| ModelSentryError::Provider {
                message: format!("HTTP request failed: {e}"),
            })?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response.text().await.unwrap_or_default();
            return Err(ModelSentryError::ProviderHttp { status, body });
        }

        let parsed: ChatResponse =
            response
                .json()
                .await
                .map_err(|e| ModelSentryError::Provider {
                    message: format!("failed to deserialize chat response: {e}"),
                })?;

        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| ModelSentryError::Provider {
                message: "no choices in OpenAI response".into(),
            })
    }

    fn provider_name(&self) -> &'static str {
        modelsentry_common::constants::provider::OPENAI
    }

    fn embedding_dim(&self) -> usize {
        self.embed_dim
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn make_provider(base_url: &str) -> OpenAiProvider {
        OpenAiProvider::new(
            reqwest::Client::new(),
            ApiKey::new("test-key".into()),
            defaults::openai::MODEL,
        )
        .expect("valid provider config")
        .with_base_url(base_url.to_string())
    }

    fn chat_ok(text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        })
    }

    fn embed_ok(vecs: Vec<Vec<f32>>) -> serde_json::Value {
        let data: Vec<_> = vecs
            .into_iter()
            .enumerate()
            .map(|(i, v)| serde_json::json!({"object": "embedding", "index": i, "embedding": v}))
            .collect();
        serde_json::json!({"object": "list", "data": data, "model": "text-embedding-3-small", "usage": {}})
    }

    #[tokio::test]
    async fn complete_parses_openai_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_ok("Hello!")))
            .mount(&server)
            .await;

        let result = make_provider(&server.uri())
            .complete("Say hello")
            .await
            .unwrap();
        assert_eq!(result, "Hello!");
    }

    #[tokio::test]
    async fn complete_sends_max_completion_tokens_not_max_tokens() {
        use wiremock::matchers::body_partial_json;
        let server = MockServer::start().await;
        // GPT-5 models 400 on `max_tokens`. The mock only matches when the body
        // carries `max_completion_tokens`, so a regression to the old field name
        // would 404 here → unwrap panics.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(
                serde_json::json!({ "max_completion_tokens": 7 }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_ok("ok")))
            .mount(&server)
            .await;

        OpenAiProvider::new(
            reqwest::Client::new(),
            ApiKey::new("test-key".into()),
            defaults::openai::MODEL,
        )
        .expect("valid provider config")
        .with_base_url(server.uri())
        .with_max_tokens(7)
        .complete("hi")
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn embed_returns_embeddings() {
        let server = MockServer::start().await;
        let vec1 = vec![0.1_f32, 0.2, 0.3];
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embed_ok(vec![vec1.clone()])))
            .mount(&server)
            .await;

        // The mock returns a 3-dim vector, so the provider must be configured to
        // expect 3 dims (the dimension guard rejects a width mismatch).
        let result = make_provider(&server.uri())
            .with_embedding_model("text-embedding-3-small", 3)
            .embed(&["hello world".into()])
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].as_slice(), vec1.as_slice());
    }

    #[tokio::test]
    async fn embed_rejects_dimension_mismatch() {
        let server = MockServer::start().await;
        // API returns a 3-dim vector, but the provider expects the default 1536.
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embed_ok(vec![vec![0.1_f32, 0.2, 0.3]])),
            )
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .embed(&["text".into()])
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ModelSentryError::DimensionMismatch {
                expected: 1536,
                actual: 3,
            }
        ));
    }

    #[tokio::test]
    async fn complete_returns_error_on_429() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .complete("hello")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ModelSentryError::ProviderHttp { status: 429, .. }
        ));
    }

    #[tokio::test]
    async fn bearer_auth_header_is_set() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_ok("ok")))
            .mount(&server)
            .await;

        make_provider(&server.uri())
            .complete("hello")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn embed_returns_error_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .embed(&["text".into()])
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ModelSentryError::ProviderHttp { status: 401, .. }
        ));
    }
}
