//! Vault key management endpoints.
//!
//! These routes store and remove LLM provider API keys (secrets) in the
//! age-encrypted vault. Providers are constructed per run from the vault + each
//! probe's [`ProviderSpec`](modelsentry_common::models::ProviderSpec), so a key
//! stored here takes effect on the next run with no restart and nothing to
//! register. The vault is keyed by provider type
//! ([`ProviderSpec::provider_id`](modelsentry_common::models::ProviderSpec::provider_id)),
//! e.g. `openai`, `anthropic`, `azure`. (Ollama needs no key.)
//!
//! # Routes
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/vault/keys` | List provider IDs with a stored key |
//! | `PUT` | `/vault/keys/{provider}` | Store or replace a key |
//! | `DELETE` | `/vault/keys/{provider}` | Remove a key |

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use modelsentry_common::{error::ModelSentryError, types::ApiKey};
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
    /// The API key (secret) for this provider.
    pub key: String,
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
    state.vault.set_key(&provider_id, &api_key).map_err(|e| {
        AppError(ModelSentryError::Vault {
            message: e.to_string(),
        })
    })?;
    tracing::info!(provider = %provider_id, "provider key stored — effective on next run");

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
        tracing::info!(provider = %provider_id, "provider key removed");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError(ModelSentryError::Config {
            message: format!("no key found for provider '{provider_id}'"),
        }))
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

    use crate::server::AppState;
    use modelsentry_common::config::{
        AlertsConfig, AuthConfig, DatabaseConfig, ProvidersConfig, SchedulerConfig, ServerConfig,
        VaultConfig,
    };
    use modelsentry_core::{
        alert::AlertEngine,
        drift::{assessment::AssessmentConfig, calculator::DriftCalculator},
    };
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
            calculator: Arc::new(DriftCalculator::new(AssessmentConfig::default())),
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
            alerts: AlertsConfig::default(),
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
    async fn upsert_key_stores_provider_key() {
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
    async fn upsert_key_rejects_empty_key() {
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
    async fn upsert_arbitrary_provider_id_stores_key() {
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
