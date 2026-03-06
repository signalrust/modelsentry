//! Anthropic Messages API adapter.
//!
//! Implements [`LlmProvider`] for the `claude-*` model family via the
//! `/v1/messages` endpoint.
//!
//! **Embedding support:** Anthropic has no native embedding endpoint.
//! [`AnthropicProvider::embed`] always returns
//! [`ModelSentryError::Provider`] with a clear message so callers can use
//! `ProbeRunner::run_completions_only` instead.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use modelsentry_common::{
    error::{ModelSentryError, Result},
    types::ApiKey,
};

use super::LlmProvider;
use crate::drift::Embedding;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 1024;

// ── Public type ───────────────────────────────────────────────────────────────

/// Anthropic Messages API adapter.
///
/// Created once and shared via [`super::DynProvider`].  The inner
/// [`reqwest::Client`] is already connection-pooled and cheap to clone.
#[derive(Debug)]
pub struct AnthropicProvider {
    api_key: ApiKey,
    client: reqwest::Client,
    model: String,
    base_url: String,
}

impl AnthropicProvider {
    /// Create a provider with a 30-second request timeout.
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
            base_url: DEFAULT_BASE_URL.to_string(),
        })
    }

    /// Override the API base URL.
    ///
    /// Primarily for tests that point the adapter at a local mock server.
    #[must_use]
    pub fn with_base_url(self, base_url: String) -> Self {
        Self { base_url, ..self }
    }
}

// ── Wire types (private) ──────────────────────────────────────────────────────

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<RequestMessage<'a>>,
}

#[derive(Serialize)]
struct RequestMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

// ── LlmProvider impl ──────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for AnthropicProvider {
    /// Always returns [`ModelSentryError::Provider`] — Anthropic has no
    /// native embedding endpoint.
    ///
    /// # Errors
    ///
    /// Always returns `ModelSentryError::Provider` with message
    /// `"embeddings not supported by Anthropic provider"`.
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Embedding>> {
        Err(ModelSentryError::Provider {
            message: "embeddings not supported by Anthropic provider".into(),
        })
    }

    /// Send a single-turn message and return the first text content block.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Provider`] on network or parse failure.
    /// - [`ModelSentryError::ProviderHttp`] when the API returns a non-200
    ///   status (e.g. 429 rate-limit, 529 overloaded).
    async fn complete(&self, prompt: &str) -> Result<String> {
        let url = format!("{}/v1/messages", self.base_url);

        let request_body = MessagesRequest {
            model: &self.model,
            max_tokens: DEFAULT_MAX_TOKENS,
            messages: vec![RequestMessage {
                role: "user",
                content: prompt,
            }],
        };

        let response = self
            .client
            .post(&url)
            .header("x-api-key", self.api_key.expose())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
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

        let parsed: MessagesResponse =
            response
                .json()
                .await
                .map_err(|e| ModelSentryError::Provider {
                    message: format!("failed to deserialize response: {e}"),
                })?;

        parsed
            .content
            .into_iter()
            .find(|b| b.block_type == "text")
            .and_then(|b| b.text)
            .ok_or_else(|| ModelSentryError::Provider {
                message: "no text content block in Anthropic response".into(),
            })
    }

    fn provider_name(&self) -> &'static str {
        "anthropic"
    }

    fn embedding_dim(&self) -> usize {
        0
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn make_provider(base_url: &str) -> AnthropicProvider {
        AnthropicProvider::new(ApiKey::new("test-key".into()), "claude-3-7-sonnet-20250219")
            .expect("valid provider config")
            .with_base_url(base_url.to_string())
    }

    fn ok_response(text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": text}],
            "model": "claude-3-7-sonnet-20250219",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 20}
        })
    }

    #[tokio::test]
    async fn complete_parses_anthropic_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response("Hello!")))
            .mount(&server)
            .await;

        let result = make_provider(&server.uri())
            .complete("Say hello")
            .await
            .unwrap();
        assert_eq!(result, "Hello!");
    }

    #[tokio::test]
    async fn embed_returns_not_supported_error() {
        let err = make_provider("http://unused")
            .embed(&["text".into()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("embeddings not supported"));
    }

    #[tokio::test]
    async fn complete_returns_error_on_overloaded_529() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(529).set_body_string("overloaded"))
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .complete("hello")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ModelSentryError::ProviderHttp { status: 529, .. }
        ));
    }

    #[tokio::test]
    async fn anthropic_version_header_is_set() {
        let server = MockServer::start().await;
        // Mock only matches when the correct version header is present.
        // A missing or wrong header produces no match → wiremock returns 404 → unwrap() panics.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response("ok")))
            .mount(&server)
            .await;

        make_provider(&server.uri()).complete("test").await.unwrap();
    }

    #[tokio::test]
    async fn new_rejects_empty_model_string() {
        let err = AnthropicProvider::new(ApiKey::new("key".into()), "").unwrap_err();
        assert!(err.to_string().contains("model name must not be empty"));
    }

    #[tokio::test]
    async fn provider_name_is_anthropic() {
        let p = make_provider("http://unused");
        assert_eq!(p.provider_name(), "anthropic");
    }

    #[tokio::test]
    async fn embedding_dim_is_zero() {
        let p = make_provider("http://unused");
        assert_eq!(p.embedding_dim(), 0);
    }
}
