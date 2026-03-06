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

use super::LlmProvider;
use crate::drift::Embedding;

const DEFAULT_BASE_URL: &str = "https://api.openai.com";
/// Default embedding model — 1536-dimensional output.
const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";
const DEFAULT_EMBEDDING_DIM: usize = 1536;
const DEFAULT_MAX_TOKENS: u32 = 1024;

// ── Public type ───────────────────────────────────────────────────────────────

/// `OpenAI` Chat Completions + Embeddings API adapter.
///
/// Created once and shared via [`super::DynProvider`]. The inner
/// [`reqwest::Client`] is already connection-pooled and cheap to clone.
#[derive(Debug)]
pub struct OpenAiProvider {
    api_key: ApiKey,
    client: reqwest::Client,
    model: String,
    embedding_model: String,
    embed_dim: usize,
    base_url: String,
}

impl OpenAiProvider {
    /// Create a provider with a 30-second request timeout and default
    /// embedding model (`text-embedding-3-small`, 1536 dims).
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Config`] if `model` is empty.
    /// - [`ModelSentryError::Provider`] if the HTTP client cannot be built.
    pub fn new(api_key: ApiKey, model: impl Into<String>) -> Result<Self> {
        let model = model.into();
        if model.is_empty() {
            return Err(ModelSentryError::Config {
                message: "model name must not be empty".into(),
            });
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ModelSentryError::Provider {
                message: format!("failed to build HTTP client: {e}"),
            })?;

        Ok(Self {
            api_key,
            client,
            model,
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
            embed_dim: DEFAULT_EMBEDDING_DIM,
            base_url: DEFAULT_BASE_URL.to_string(),
        })
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
    max_tokens: u32,
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
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        let url = format!("{}/v1/embeddings", self.base_url);

        let request_body = EmbedRequest {
            model: &self.embedding_model,
            input: texts,
        };

        let response = self
            .client
            .post(&url)
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
            .map(|d| Embedding::new(d.embedding))
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
            max_tokens: DEFAULT_MAX_TOKENS,
            messages: vec![ChatMessage {
                role: "user",
                content: prompt,
            }],
        };

        let response = self
            .client
            .post(&url)
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
        "openai"
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
        OpenAiProvider::new(ApiKey::new("test-key".into()), "gpt-4o")
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
    async fn embed_returns_embeddings() {
        let server = MockServer::start().await;
        let vec1 = vec![0.1_f32, 0.2, 0.3];
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(embed_ok(vec![vec1.clone()])))
            .mount(&server)
            .await;

        let result = make_provider(&server.uri())
            .embed(&["hello world".into()])
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].as_slice(), vec1.as_slice());
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
