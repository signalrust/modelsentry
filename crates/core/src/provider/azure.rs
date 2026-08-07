//! Azure OpenAI Chat Completions + Embeddings API adapter (v1 API).
//!
//! Targets the Azure OpenAI **v1 API** (GA since August 2025), which is
//! OpenAI-shaped but differs in three ways the adapter honors:
//! - **URL layout:** one versionless base path, `{endpoint}/openai/v1/{op}`,
//!   with a single stable `api-version=preview` channel (no dated monthly
//!   versions). The model is **not** in the path: the deployment name is sent
//!   as the `model` field of the request body, exactly like OpenAI.
//! - **Auth:** an `api-key` request header, not `Authorization: Bearer`.
//! - **Model identity:** the response echoes the underlying model in its
//!   `model` field, which the adapter surfaces for model-version drift checks.
//!
//! The resource `endpoint`, `api_version`, and embedding deployment come from
//! `[providers.azure]`; the chat deployment comes from the per-probe
//! [`ProviderSpec`](modelsentry_common::models::ProviderSpec). When no embedding
//! deployment is configured, [`embedding_dim`](LlmProvider::embedding_dim)
//! returns `0` so callers fall back to completions-only.
//
// "OpenAI"/"Azure OpenAI" are brand names, not code items — exempt from the lint.
#![allow(clippy::doc_markdown)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use modelsentry_common::{
    constants::defaults::azure as azure_defaults,
    error::{ModelSentryError, Result},
    types::ApiKey,
};

use super::{Completion, LlmProvider, reject_unusable_finish_reason};
use crate::drift::Embedding;

// ── Public type ───────────────────────────────────────────────────────────────

/// Azure OpenAI Chat Completions + Embeddings API adapter.
///
/// Built per run from a **shared** [`reqwest::Client`] (injected, so the
/// connection pool is reused across runs rather than rebuilt each time). The
/// per-request timeout is applied on each call.
#[derive(Debug)]
pub struct AzureOpenAiProvider {
    api_key: ApiKey,
    client: reqwest::Client,
    request_timeout: std::time::Duration,
    /// Resource endpoint with any trailing slash trimmed.
    endpoint: String,
    chat_deployment: String,
    embedding_deployment: Option<String>,
    embed_dim: usize,
    api_version: String,
    max_tokens: u32,
}

impl AzureOpenAiProvider {
    /// Create a provider for an Azure OpenAI resource over the shared `client`.
    ///
    /// `endpoint` is the resource root (e.g.
    /// `https://my-resource.openai.azure.com`); a trailing slash is trimmed.
    ///
    /// # Errors
    ///
    /// [`ModelSentryError::Config`] if `endpoint`, `chat_deployment`, or
    /// `api_version` is empty.
    pub fn new(
        client: reqwest::Client,
        api_key: ApiKey,
        endpoint: impl Into<String>,
        chat_deployment: impl Into<String>,
        api_version: impl Into<String>,
    ) -> Result<Self> {
        let endpoint = endpoint.into().trim_end_matches('/').to_string();
        let chat_deployment = chat_deployment.into();
        let api_version = api_version.into();
        if endpoint.is_empty() {
            return Err(ModelSentryError::Config {
                message: "Azure endpoint must not be empty — set [providers.azure] endpoint".into(),
            });
        }
        if chat_deployment.is_empty() {
            return Err(ModelSentryError::Config {
                message: "Azure chat deployment must not be empty".into(),
            });
        }
        if api_version.is_empty() {
            return Err(ModelSentryError::Config {
                message: "Azure api_version must not be empty".into(),
            });
        }

        Ok(Self {
            api_key,
            client,
            request_timeout: std::time::Duration::from_secs(azure_defaults::TIMEOUT_SECS),
            endpoint,
            chat_deployment,
            embedding_deployment: None,
            embed_dim: 0,
            api_version,
            max_tokens: modelsentry_common::constants::defaults::MAX_TOKENS,
        })
    }

    /// Configure the embedding deployment and its output dimension. Without
    /// this, the provider reports no embedding support (drift disabled).
    #[must_use]
    pub fn with_embedding(self, deployment: Option<String>, dim: usize) -> Self {
        Self {
            embedding_deployment: deployment,
            embed_dim: dim,
            ..self
        }
    }

    /// Override the per-request `max_tokens` (completion length cap).
    #[must_use]
    pub fn with_max_tokens(self, max_tokens: u32) -> Self {
        Self { max_tokens, ..self }
    }

    /// Build the v1 URL for an operation (`chat/completions`, `embeddings`):
    /// `{endpoint}/openai/v1/{operation}?api-version={api_version}`. The model
    /// is supplied as the request body's `model` field, not in the path.
    fn v1_url(&self, operation: &str) -> String {
        format!(
            "{}/openai/v1/{operation}?api-version={}",
            self.endpoint, self.api_version
        )
    }
}

// ── Wire types (private) ──────────────────────────────────────────────────────
//
// The v1 API takes the deployment name as the request body's `model` field
// (OpenAI-shaped), not in the URL path.

#[derive(Serialize)]
struct ChatRequest<'a> {
    /// The chat deployment name (Azure's `model` is the deployment).
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    // Modern Azure api-versions accept `max_completion_tokens` (and reasoning
    // models require it); mirrors the OpenAI adapter.
    max_completion_tokens: u32,
    // Cache-busting (exchangeability): don't persist this completion for reuse,
    // and tag each request with a fresh non-semantic nonce so an exact-request
    // cache cannot return identical completions across baseline runs.
    store: bool,
    user: &'a str,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    /// Underlying model identity echoed by the API, used to detect a silent
    /// model-version change since baseline capture.
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageContent,
    /// Why generation stopped: `"stop"` (complete), `"length"` (truncated),
    /// `"content_filter"` (blocked) — the latter two are not usable completions.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessageContent {
    content: String,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    /// The embedding deployment name (the v1 embeddings route requires `model`).
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
impl LlmProvider for AzureOpenAiProvider {
    /// Embed a batch of texts via the configured embedding deployment.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Provider`] if no embedding deployment is configured.
    /// - [`ModelSentryError::ProviderTransport`] on a network/transport failure.
    /// - [`ModelSentryError::ProviderDecode`] if the response cannot be decoded.
    /// - [`ModelSentryError::ProviderHttp`] on a non-200 HTTP status.
    /// - [`ModelSentryError::DimensionMismatch`] if a returned embedding width
    ///   differs from the configured `embedding_dim`.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        let deployment =
            self.embedding_deployment
                .as_deref()
                .ok_or_else(|| ModelSentryError::Provider {
                    message: "no Azure embedding deployment configured — set \
                          [providers.azure] embedding_deployment to enable drift detection"
                        .into(),
                })?;
        let url = self.v1_url("embeddings");

        let response = self
            .client
            .post(&url)
            .timeout(self.request_timeout)
            .header(azure_defaults::API_KEY_HEADER, self.api_key.expose())
            .json(&EmbedRequest {
                model: deployment,
                input: texts,
            })
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

        let parsed: EmbedResponse =
            response
                .json()
                .await
                .map_err(|e| ModelSentryError::ProviderDecode {
                    message: format!("embeddings response: {e}"),
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

    /// Send a single-turn chat message to the chat deployment and return the
    /// assistant reply.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::ProviderTransport`] on a network/transport failure.
    /// - [`ModelSentryError::ProviderDecode`] if the response cannot be decoded.
    /// - [`ModelSentryError::ProviderHttp`] on a non-200 HTTP status.
    async fn complete(&self, prompt: &str) -> Result<Completion> {
        let url = self.v1_url("chat/completions");

        let nonce = super::cache_bust_nonce();
        let response = self
            .client
            .post(&url)
            .timeout(self.request_timeout)
            .header(azure_defaults::API_KEY_HEADER, self.api_key.expose())
            .json(&ChatRequest {
                model: &self.chat_deployment,
                max_completion_tokens: self.max_tokens,
                store: false,
                user: &nonce,
                messages: vec![ChatMessage {
                    role: "user",
                    content: prompt,
                }],
            })
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

        let parsed: ChatResponse =
            response
                .json()
                .await
                .map_err(|e| ModelSentryError::ProviderDecode {
                    message: format!("chat response: {e}"),
                })?;

        let model_version = parsed.model.filter(|m| !m.is_empty());
        let choice =
            parsed
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| ModelSentryError::Provider {
                    message: "no choices in Azure OpenAI response".into(),
                })?;
        reject_unusable_finish_reason(choice.finish_reason.as_deref())?;
        Ok(Completion {
            text: choice.message.content,
            model_version,
        })
    }

    fn provider_name(&self) -> &'static str {
        modelsentry_common::constants::provider::AZURE
    }

    /// The configured embedding dimension, or `0` when no embedding deployment
    /// is set (gates the has-embeddings capability check).
    fn embedding_dim(&self) -> usize {
        if self.embedding_deployment.is_some() {
            self.embed_dim
        } else {
            0
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn make_provider(base_url: &str) -> AzureOpenAiProvider {
        AzureOpenAiProvider::new(
            reqwest::Client::new(),
            ApiKey::new("test-key".into()),
            base_url,
            "gpt-4o-prod",
            azure_defaults::API_VERSION,
        )
        .expect("valid provider config")
    }

    fn chat_ok(text: &str) -> serde_json::Value {
        serde_json::json!({
            "model": azure_defaults::MODELS[0],
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}]
        })
    }

    fn embed_ok(vecs: Vec<Vec<f32>>) -> serde_json::Value {
        let data: Vec<_> = vecs
            .into_iter()
            .enumerate()
            .map(|(i, v)| serde_json::json!({"object": "embedding", "index": i, "embedding": v}))
            .collect();
        serde_json::json!({"object": "list", "data": data})
    }

    #[tokio::test]
    async fn complete_uses_v1_path_deployment_as_model_and_api_key_header() {
        use wiremock::matchers::body_partial_json;
        let server = MockServer::start().await;
        // v1 API: versionless `/openai/v1/...` path, `api-version=preview`
        // channel, and the deployment carried as the body `model` field.
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .and(query_param("api-version", azure_defaults::API_VERSION))
            .and(header("api-key", "test-key"))
            .and(body_partial_json(
                serde_json::json!({ "model": "gpt-4o-prod" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_ok("Hello!")))
            .mount(&server)
            .await;

        let result = make_provider(&server.uri())
            .complete("Say hello")
            .await
            .unwrap();
        assert_eq!(result.text, "Hello!");
        assert_eq!(
            result.model_version.as_deref(),
            Some(azure_defaults::MODELS[0])
        );
    }

    #[tokio::test]
    async fn trailing_slash_in_endpoint_is_trimmed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_ok("ok")))
            .mount(&server)
            .await;

        AzureOpenAiProvider::new(
            reqwest::Client::new(),
            ApiKey::new("test-key".into()),
            format!("{}/", server.uri()),
            "gpt-4o-prod",
            azure_defaults::API_VERSION,
        )
        .expect("valid config")
        .complete("hi")
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn complete_sends_cache_busting_store_false() {
        use wiremock::matchers::body_partial_json;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({ "store": false })))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_ok("ok")))
            .mount(&server)
            .await;

        make_provider(&server.uri()).complete("hi").await.unwrap();
    }

    #[tokio::test]
    async fn complete_rejects_truncated_completion() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": azure_defaults::MODELS[0],
                "choices": [{"index": 0, "finish_reason": "length",
                             "message": {"role": "assistant", "content": "partial"}}]
            })))
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .complete("hi")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    #[tokio::test]
    async fn embed_hits_v1_route_with_deployment_as_model() {
        use wiremock::matchers::body_partial_json;
        let server = MockServer::start().await;
        let vec1 = vec![0.1_f32, 0.2, 0.3];
        Mock::given(method("POST"))
            .and(path("/openai/v1/embeddings"))
            .and(header("api-key", "test-key"))
            .and(body_partial_json(
                serde_json::json!({ "model": "embed-prod" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(embed_ok(vec![vec1.clone()])))
            .mount(&server)
            .await;

        let result = make_provider(&server.uri())
            .with_embedding(Some("embed-prod".to_string()), 3)
            .embed(&["hello world".into()])
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].as_slice(), vec1.as_slice());
    }

    #[tokio::test]
    async fn embed_without_deployment_is_an_error_and_dim_is_zero() {
        let provider = make_provider("https://example.openai.azure.com");
        assert_eq!(provider.embedding_dim(), 0);
        let err = provider.embed(&["text".into()]).await.unwrap_err();
        assert!(matches!(err, ModelSentryError::Provider { .. }));
    }

    #[tokio::test]
    async fn embed_rejects_dimension_mismatch() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embed_ok(vec![vec![0.1_f32, 0.2, 0.3]])),
            )
            .mount(&server)
            .await;

        let err = make_provider(&server.uri())
            .with_embedding(Some("embed-prod".to_string()), 1536)
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
            .and(path("/openai/v1/chat/completions"))
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

    #[test]
    fn empty_endpoint_is_rejected() {
        let err = AzureOpenAiProvider::new(
            reqwest::Client::new(),
            ApiKey::new("k".into()),
            "",
            "gpt-4o-prod",
            azure_defaults::API_VERSION,
        )
        .unwrap_err();
        assert!(err.to_string().contains("endpoint"));
    }

    #[test]
    fn empty_chat_deployment_is_rejected() {
        let err = AzureOpenAiProvider::new(
            reqwest::Client::new(),
            ApiKey::new("k".into()),
            "https://x.example",
            "",
            azure_defaults::API_VERSION,
        )
        .unwrap_err();
        assert!(err.to_string().contains("deployment"));
    }
}
