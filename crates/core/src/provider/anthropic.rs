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

use modelsentry_common::constants::defaults;

use super::{Completion, LlmProvider};
use crate::drift::Embedding;

// ── Public type ───────────────────────────────────────────────────────────────

/// Anthropic Messages API adapter.
///
/// Built per run from a **shared** [`reqwest::Client`] (injected, so the
/// connection pool is reused across runs rather than rebuilt each time). The
/// per-request timeout is applied on each call.
#[derive(Debug)]
pub struct AnthropicProvider {
    api_key: ApiKey,
    client: reqwest::Client,
    request_timeout: std::time::Duration,
    model: String,
    base_url: String,
    max_tokens: u32,
}

impl AnthropicProvider {
    /// Create a provider over the shared `client`, with a 30-second per-request
    /// timeout.
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
            request_timeout: std::time::Duration::from_secs(defaults::anthropic::TIMEOUT_SECS),
            model,
            base_url: defaults::anthropic::BASE_URL.to_string(),
            max_tokens: defaults::MAX_TOKENS,
        })
    }

    /// Override the API base URL.
    ///
    /// Primarily for tests that point the adapter at a local mock server.
    #[must_use]
    pub fn with_base_url(self, base_url: String) -> Self {
        Self { base_url, ..self }
    }

    /// Override the per-request `max_tokens` (completion length cap).
    #[must_use]
    pub fn with_max_tokens(self, max_tokens: u32) -> Self {
        Self { max_tokens, ..self }
    }
}

// ── Wire types (private) ──────────────────────────────────────────────────────

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<RequestMessage<'a>>,
    // Cache-busting (exchangeability): a fresh non-semantic nonce per request,
    // so an exact-request cache cannot return identical completions across
    // baseline runs (which would collapse a prompt's baseline cloud).
    metadata: RequestMetadata<'a>,
}

#[derive(Serialize)]
struct RequestMetadata<'a> {
    user_id: &'a str,
}

#[derive(Serialize)]
struct RequestMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    /// Why generation stopped. `"refusal"` means a safety classifier declined
    /// the request (returned as HTTP 200 with empty/partial content on current
    /// Claude models) — distinct from `"end_turn"`.
    #[serde(default)]
    stop_reason: Option<String>,
    /// Resolved model identity (a pinned snapshot id, e.g. `claude-sonnet-4-6`),
    /// used to detect a silent model-version change since baseline capture.
    #[serde(default)]
    model: Option<String>,
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
    /// - [`ModelSentryError::ProviderTransport`] on a network/transport failure.
    /// - [`ModelSentryError::ProviderDecode`] if the response cannot be decoded.
    /// - [`ModelSentryError::Provider`] on a semantic failure (safety refusal,
    ///   or no text content block).
    /// - [`ModelSentryError::ProviderHttp`] when the API returns a non-200
    ///   status (e.g. 429 rate-limit, 529 overloaded).
    async fn complete(&self, prompt: &str) -> Result<Completion> {
        let url = format!("{}/v1/messages", self.base_url);

        let nonce = super::cache_bust_nonce();
        let request_body = MessagesRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            metadata: RequestMetadata { user_id: &nonce },
            messages: vec![RequestMessage {
                role: "user",
                content: prompt,
            }],
        };

        let response = self
            .client
            .post(&url)
            .timeout(self.request_timeout)
            .header(defaults::anthropic::API_KEY_HEADER, self.api_key.expose())
            .header(
                defaults::anthropic::VERSION_HEADER,
                defaults::anthropic::API_VERSION,
            )
            // `.json()` sets `content-type: application/json`.
            .json(&request_body)
            .send()
            .await
            .map_err(|e| ModelSentryError::ProviderTransport {
                message: e.to_string(),
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
                .map_err(|e| ModelSentryError::ProviderDecode {
                    message: format!("messages response: {e}"),
                })?;

        // A safety refusal returns HTTP 200 but is not a usable completion —
        // surface it explicitly so it never silently pollutes a baseline.
        if parsed.stop_reason.as_deref() == Some("refusal") {
            return Err(ModelSentryError::Provider {
                message: "Anthropic declined the request (stop_reason: refusal)".into(),
            });
        }
        // A completion truncated at the token cap is not representative of the
        // model's full output, so it must not be folded into a baseline either.
        if parsed.stop_reason.as_deref() == Some("max_tokens") {
            return Err(ModelSentryError::Provider {
                message: "completion truncated (stop_reason: max_tokens) — raise \
                          [providers.anthropic] max_tokens or shorten the prompt"
                    .into(),
            });
        }

        let model_version = parsed.model.filter(|m| !m.is_empty());
        let text = parsed
            .content
            .into_iter()
            .find(|b| b.block_type == "text")
            .and_then(|b| b.text)
            .ok_or_else(|| ModelSentryError::Provider {
                message: "no text content block in Anthropic response".into(),
            })?;
        Ok(Completion {
            text,
            model_version,
        })
    }

    fn provider_name(&self) -> &'static str {
        modelsentry_common::constants::provider::ANTHROPIC
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
        AnthropicProvider::new(
            reqwest::Client::new(),
            ApiKey::new("test-key".into()),
            defaults::anthropic::MODEL,
        )
        .expect("valid provider config")
        .with_base_url(base_url.to_string())
    }

    fn ok_response(text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": text}],
            "model": defaults::anthropic::MODEL,
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
        assert_eq!(result.text, "Hello!");
        assert_eq!(
            result.model_version.as_deref(),
            Some(defaults::anthropic::MODEL)
        );
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
    async fn complete_returns_error_on_refusal_stop_reason() {
        let server = MockServer::start().await;
        // HTTP 200 but the safety classifier declined: empty content + refusal.
        let refusal = serde_json::json!({
            "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": defaults::anthropic::MODEL,
            "stop_reason": "refusal",
            "usage": {"input_tokens": 10, "output_tokens": 0}
        });
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(refusal))
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .complete("hello")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("refusal"));
    }

    #[tokio::test]
    async fn complete_rejects_max_tokens_truncation() {
        let server = MockServer::start().await;
        // HTTP 200 but the output was cut off at the token cap (stop_reason
        // "max_tokens") — not a representative completion, so it must error.
        let truncated = serde_json::json!({
            "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "partial"}],
            "model": defaults::anthropic::MODEL,
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 10, "output_tokens": 1024}
        });
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(truncated))
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .complete("hello")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("truncated"), "{err}");
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
    async fn complete_sends_cache_busting_metadata() {
        use wiremock::matchers::body_partial_json;
        let server = MockServer::start().await;
        // The mock only matches when the request carries a `metadata` object
        // (the cache-busting user_id nonce); dropping it would 404 → panic.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(serde_json::json!({ "metadata": {} })))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response("ok")))
            .mount(&server)
            .await;

        make_provider(&server.uri()).complete("hi").await.unwrap();
    }

    #[tokio::test]
    async fn complete_sends_configured_max_tokens() {
        use wiremock::matchers::body_partial_json;
        let server = MockServer::start().await;
        // The mock only matches when the body carries the overridden value, so
        // a regression to the hardcoded default would 404 → unwrap panics.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(serde_json::json!({ "max_tokens": 7 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response("ok")))
            .mount(&server)
            .await;

        AnthropicProvider::new(
            reqwest::Client::new(),
            ApiKey::new("test-key".into()),
            defaults::anthropic::MODEL,
        )
        .expect("valid provider config")
        .with_base_url(server.uri())
        .with_max_tokens(7)
        .complete("hi")
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn new_rejects_empty_model_string() {
        let err = AnthropicProvider::new(reqwest::Client::new(), ApiKey::new("key".into()), "")
            .unwrap_err();
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
