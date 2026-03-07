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

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use modelsentry_common::{config::AppConfig, error::ModelSentryError, types::ApiKey};
use modelsentry_core::provider::DynProvider;
use serde::{Deserialize, Serialize};

use super::AppError;
use crate::provider_factory::{self, ProviderOverrides};
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
    // Ollama has no API key — the 'key' field is used as the base URL and may
    // be empty (falls back to http://localhost:11434).
    if !provider_id.starts_with("ollama") && body.key.trim().is_empty() {
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
    let dyn_provider: Option<DynProvider> =
        build_provider(&provider_id, api_key, &body, &state.config)
            .transpose()
            .map_err(|e| {
                AppError(ModelSentryError::Config {
                    message: format!("key saved but provider init failed: {e}"),
                })
            })?;

    if let Some(p) = dyn_provider {
        // Use the same registry-key convention as provider_key() in the scheduler
        // so that probe lookups always find the right entry.
        // Ollama: "ollama:{base_url}";  others: "{provider_id}".
        let registry_key = if provider_id.starts_with("ollama") {
            let base_url = if body.key.trim().is_empty() {
                body.base_url.as_deref().unwrap_or("http://localhost:11434")
            } else {
                body.key.trim()
            };
            format!("ollama:{base_url}")
        } else {
            provider_id.clone()
        };
        state
            .providers
            .write()
            .map_err(|e| {
                AppError(ModelSentryError::Config {
                    message: format!("provider registry poisoned: {e}"),
                })
            })?
            .insert(registry_key, p);
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
    // For Ollama, read the stored base_url *before* deleting so we can derive
    // the correct registry key ("ollama:{base_url}") to evict from the live map.
    let stored_base_url: Option<String> = if provider_id.starts_with("ollama") {
        state
            .vault
            .get_key(&provider_id)
            .ok()
            .flatten()
            .map(|k| k.expose().to_string())
    } else {
        None
    };

    let removed = state.vault.delete_key(&provider_id).map_err(|e| {
        AppError(ModelSentryError::Vault {
            message: e.to_string(),
        })
    })?;

    if removed {
        let registry_key = if provider_id.starts_with("ollama") {
            let base_url = stored_base_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("http://localhost:11434");
            format!("ollama:{base_url}")
        } else {
            provider_id.clone()
        };
        state
            .providers
            .write()
            .map_err(|e| {
                AppError(ModelSentryError::Config {
                    message: format!("provider registry poisoned: {e}"),
                })
            })?
            .remove(&registry_key);
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
    config: &AppConfig,
) -> Option<modelsentry_common::error::Result<DynProvider>> {
    let overrides = ProviderOverrides {
        model: body.model.clone(),
        base_url: body.base_url.clone(),
    };
    match provider_factory::build_provider(provider_id, key, &overrides, config) {
        Ok(Some(p)) => Some(Ok(p)),
        Ok(None) => None,
        Err(e) => Some(Err(e)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::{scheduler::new_registry, server::AppState};
    use modelsentry_common::config::{
        AlertsConfig, AuthConfig, DatabaseConfig, ProvidersConfig, SchedulerConfig, ServerConfig,
        VaultConfig,
    };
    use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
    use modelsentry_store::AppStore;

    fn open_store() -> (TempDir, Arc<AppStore>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        (dir, Arc::new(AppStore::open(&path).unwrap()))
    }

    fn test_app(store: Arc<AppStore>) -> (TempDir, TestServer) {
        let vault_dir = tempfile::tempdir().unwrap();
        let vault_path = vault_dir.path().join("v.age");
        let state = AppState {
            store,
            vault: Arc::new(
                crate::vault::Vault::create(
                    &vault_path,
                    secrecy::SecretString::new("test".to_string().into()),
                )
                .unwrap(),
            ),
            providers: Arc::new(new_registry()),
            calculator: Arc::new(DriftCalculator::new(0.5, 0.5).unwrap()),
            alert_engine: Arc::new(AlertEngine::default()),
            config: Arc::new(make_config()),
        };
        let router = crate::routes::router(state);
        (vault_dir, TestServer::new(router))
    }

    fn make_config() -> modelsentry_common::config::AppConfig {
        modelsentry_common::config::AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 7740,
                timeout_secs: 30,
                cors_origin: "http://localhost:5173".to_string(),
            },
            vault: VaultConfig {
                path: std::path::PathBuf::from("/tmp/vault.age"),
            },
            database: DatabaseConfig {
                path: std::path::PathBuf::from("/tmp/modelsentry.db"),
            },
            scheduler: SchedulerConfig {
                default_interval_minutes: 60,
            },
            alerts: AlertsConfig {
                drift_threshold_kl: 0.5,
                drift_threshold_cos: 0.5,
            },
            providers: ProvidersConfig::default(),
            auth: AuthConfig::default(),
        }
    }

    #[tokio::test]
    async fn list_keys_returns_empty_initially() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let resp = server.get("/vault/keys").await;
        resp.assert_status_ok();
        let body: serde_json::Value = resp.json();
        assert_eq!(body["providers"], json!([]));
    }

    #[tokio::test]
    async fn upsert_key_stores_and_registers_provider() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let resp = server
            .put("/vault/keys/openai")
            .json(&json!({ "key": "sk-test-key-12345" }))
            .await;
        resp.assert_status_ok();
        let body: serde_json::Value = resp.json();
        assert_eq!(body["provider"], "openai");
        assert_eq!(body["status"], "stored");

        // The key should now appear in list
        let list = server.get("/vault/keys").await;
        let body: serde_json::Value = list.json();
        let providers = body["providers"].as_array().unwrap();
        assert!(providers.iter().any(|p| p == "openai"));
    }

    #[tokio::test]
    async fn upsert_key_rejects_empty_key_for_non_ollama() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let resp = server
            .put("/vault/keys/openai")
            .json(&json!({ "key": "  " }))
            .await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn delete_key_returns_error_for_absent_key() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let resp = server.delete("/vault/keys/nonexistent").await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn upsert_then_delete_key_round_trip() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);

        // Upsert
        let resp = server
            .put("/vault/keys/anthropic")
            .json(&json!({ "key": "sk-anthropic-key" }))
            .await;
        resp.assert_status_ok();

        // Delete
        let resp = server.delete("/vault/keys/anthropic").await;
        resp.assert_status(StatusCode::NO_CONTENT);

        // List should be empty again
        let list = server.get("/vault/keys").await;
        let body: serde_json::Value = list.json();
        assert_eq!(body["providers"], json!([]));
    }

    #[tokio::test]
    async fn upsert_ollama_allows_empty_key() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let resp = server
            .put("/vault/keys/ollama")
            .json(&json!({ "key": "", "base_url": "http://localhost:11434" }))
            .await;
        resp.assert_status_ok();
    }

    #[tokio::test]
    async fn upsert_unknown_provider_stores_key_without_registering() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let resp = server
            .put("/vault/keys/custom-provider")
            .json(&json!({ "key": "some-api-key" }))
            .await;
        resp.assert_status_ok();

        // Key should be listed
        let list = server.get("/vault/keys").await;
        let body: serde_json::Value = list.json();
        let providers = body["providers"].as_array().unwrap();
        assert!(providers.iter().any(|p| p == "custom-provider"));
    }
}
