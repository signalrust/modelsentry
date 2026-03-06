//! Vault key management endpoints.
//!
//! These routes allow storing and removing LLM provider API keys in the
//! age-encrypted vault without restarting the daemon.  The in-memory
//! [`ProviderRegistry`] is updated atomically so new or deleted keys take
//! effect immediately for future scheduled and on-demand probe runs.
//!
//! # Routes
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/vault/keys` | List registered provider IDs |
//! | `PUT` | `/vault/keys/{provider}` | Store or replace a key |
//! | `DELETE` | `/vault/keys/{provider}` | Remove a key |

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use modelsentry_common::{error::ModelSentryError, types::ApiKey};
use modelsentry_core::provider::{
    DynProvider, anthropic::AnthropicProvider, ollama::OllamaProvider, openai::OpenAiProvider,
};
use serde::{Deserialize, Serialize};

use super::AppError;
use crate::server::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vault/keys", get(list_keys))
        .route("/vault/keys/{provider}", put(upsert_key).delete(delete_key))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct UpsertKeyRequest {
    /// The API key value.
    pub key: String,
    /// Optional model override used when constructing the provider.
    /// Defaults to the provider's recommended default model.
    pub model: Option<String>,
    /// For Ollama: base URL of the local Ollama server (default:
    /// `http://localhost:11434`).
    pub base_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct KeyListResponse {
    pub providers: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UpsertKeyResponse {
    pub provider: String,
    pub status: &'static str,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_keys(State(state): State<AppState>) -> Result<Json<KeyListResponse>, AppError> {
    let providers = state.vault.list_providers().map_err(|e| {
        AppError(ModelSentryError::Vault {
            message: e.to_string(),
        })
    })?;
    Ok(Json(KeyListResponse { providers }))
}

async fn upsert_key(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(body): Json<UpsertKeyRequest>,
) -> Result<(StatusCode, Json<UpsertKeyResponse>), AppError> {
    if body.key.trim().is_empty() {
        return Err(AppError(ModelSentryError::Config {
            message: "key must not be empty".to_string(),
        }));
    }

    let api_key = ApiKey::new(body.key.clone());

    // Persist to vault.
    state.vault.set_key(&provider_id, &api_key).map_err(|e| {
        AppError(ModelSentryError::Vault {
            message: e.to_string(),
        })
    })?;

    // Construct and register the provider in the live registry so the
    // change takes effect immediately without a restart.
    let dyn_provider: Option<DynProvider> = build_provider(&provider_id, api_key, &body)
        .transpose()
        .map_err(|e| {
            AppError(ModelSentryError::Config {
                message: format!("key saved but provider init failed: {e}"),
            })
        })?;

    if let Some(p) = dyn_provider {
        state
            .providers
            .write()
            .unwrap()
            .insert(provider_id.clone(), p);
        tracing::info!(provider = %provider_id, "provider registered via vault API");
    } else {
        tracing::info!(
            provider = %provider_id,
            "key stored in vault (unknown provider type — will take effect on restart)"
        );
    }

    Ok((
        StatusCode::OK,
        Json(UpsertKeyResponse {
            provider: provider_id,
            status: "stored",
        }),
    ))
}

async fn delete_key(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let removed = state.vault.delete_key(&provider_id).map_err(|e| {
        AppError(ModelSentryError::Vault {
            message: e.to_string(),
        })
    })?;

    if removed {
        state.providers.write().unwrap().remove(&provider_id);
        tracing::info!(provider = %provider_id, "provider removed via vault API");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError(ModelSentryError::Config {
            message: format!("no key found for provider '{provider_id}'"),
        }))
    }
}

// ---------------------------------------------------------------------------
// Internal helper
// ---------------------------------------------------------------------------

/// Attempt to construct a typed provider from the known provider IDs.
/// Returns `None` for unknown IDs (key is still saved; provider won't be
/// available until restart).
fn build_provider(
    provider_id: &str,
    key: ApiKey,
    body: &UpsertKeyRequest,
) -> Option<modelsentry_common::error::Result<DynProvider>> {
    match provider_id {
        "openai" => {
            let model = body.model.as_deref().unwrap_or("gpt-4o").to_string();
            Some(OpenAiProvider::new(key, model).map(|p| Arc::new(p) as DynProvider))
        }
        "anthropic" => {
            let model = body
                .model
                .as_deref()
                .unwrap_or("claude-3-7-sonnet-20250219")
                .to_string();
            Some(AnthropicProvider::new(key, model).map(|p| Arc::new(p) as DynProvider))
        }
        id if id.starts_with("ollama") => {
            let base_url = body
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let model = body.model.as_deref().unwrap_or("llama3").to_string();
            Some(OllamaProvider::new(model, base_url).map(|p| Arc::new(p) as DynProvider))
        }
        _ => None,
    }
}
