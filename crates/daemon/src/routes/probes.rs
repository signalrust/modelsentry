//! Probe CRUD + run-now endpoints.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::Utc;
use modelsentry_common::{
    error::ModelSentryError,
    models::{Probe, ProbePrompt, ProbeRun, ProbeSchedule, ProviderKind},
    types::ProbeId,
};
use serde::Deserialize;

use crate::{routes::AppError, server::AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/probes", get(list_probes).post(create_probe))
        .route("/probes/{id}", get(get_probe).delete(delete_probe))
        .route("/probes/{id}/run-now", post(trigger_probe_run))
}

// ---------------------------------------------------------------------------
// Request body
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateProbeRequest {
    pub name: String,
    pub provider: ProviderKind,
    pub model: String,
    pub prompts: Vec<ProbePrompt>,
    pub schedule: ProbeSchedule,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_probes(State(state): State<AppState>) -> Result<Json<Vec<Probe>>, AppError> {
    let probes = state.store.probes().list_all()?;
    Ok(Json(probes))
}

async fn create_probe(
    State(state): State<AppState>,
    Json(body): Json<CreateProbeRequest>,
) -> Result<(StatusCode, Json<Probe>), AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError(ModelSentryError::Config {
            message: "name must not be empty".to_string(),
        }));
    }
    if body.model.trim().is_empty() {
        return Err(AppError(ModelSentryError::Config {
            message: "model must not be empty".to_string(),
        }));
    }
    if body.prompts.is_empty() {
        return Err(AppError(ModelSentryError::Config {
            message: "prompts must not be empty".to_string(),
        }));
    }
    if body.prompts.iter().any(|p| p.text.trim().is_empty()) {
        return Err(AppError(ModelSentryError::Config {
            message: "each prompt text must not be empty".to_string(),
        }));
    }
    let now = Utc::now();
    let probe = Probe {
        id: ProbeId::new(),
        name: body.name,
        provider: body.provider,
        model: body.model,
        prompts: body.prompts,
        schedule: body.schedule,
        created_at: now,
        updated_at: now,
    };
    state.store.probes().insert(&probe)?;
    Ok((StatusCode::CREATED, Json(probe)))
}

async fn get_probe(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Probe>, AppError> {
    let probe_id = parse_probe_id(&id)?;
    state
        .store
        .probes()
        .get(&probe_id)?
        .map(Json)
        .ok_or_else(|| AppError(ModelSentryError::ProbeNotFound { id: id.clone() }))
}

async fn delete_probe(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let probe_id = parse_probe_id(&id)?;
    let found = state.store.delete_probe_cascade(&probe_id)?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError(ModelSentryError::ProbeNotFound { id }))
    }
}

async fn trigger_probe_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProbeRun>, AppError> {
    use modelsentry_core::probe_runner::ProbeRunner;

    let probe_id = parse_probe_id(&id)?;
    let probe = state
        .store
        .probes()
        .get(&probe_id)?
        .ok_or_else(|| AppError(ModelSentryError::ProbeNotFound { id: id.clone() }))?;

    let provider_key = provider_key_for(&probe.provider);
    let provider = state
        .providers
        .read()
        .map_err(|e| {
            AppError(ModelSentryError::Config {
                message: format!("provider registry poisoned: {e}"),
            })
        })?
        .get(&provider_key)
        .cloned()
        .ok_or_else(|| {
            AppError(ModelSentryError::Config {
                message: format!("no provider registered for '{provider_key}'"),
            })
        })?;

    let runner = ProbeRunner::new(provider);
    let concurrency = 4;
    let run = if runner.has_embeddings() {
        runner.run(&probe, concurrency).await?
    } else {
        runner.run_completions_only(&probe, concurrency).await?
    };
    state.store.runs().insert(&run)?;
    Ok(Json(run))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_probe_id(s: &str) -> Result<ProbeId, AppError> {
    let uuid = uuid::Uuid::parse_str(s)
        .map_err(|_| AppError(ModelSentryError::ProbeNotFound { id: s.to_string() }))?;
    Ok(ProbeId::from_uuid(uuid))
}

fn provider_key_for(kind: &ProviderKind) -> String {
    match kind {
        ProviderKind::OpenAi => "openai".to_string(),
        ProviderKind::Anthropic => "anthropic".to_string(),
        ProviderKind::Ollama { base_url } => format!("ollama:{base_url}"),
        ProviderKind::AzureOpenAi {
            endpoint,
            deployment,
        } => format!("azure:{endpoint}:{deployment}"),
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
    use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
    use modelsentry_store::AppStore;

    fn open_store() -> (TempDir, Arc<AppStore>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        (dir, Arc::new(AppStore::open(&path).unwrap()))
    }

    fn test_app(store: Arc<AppStore>) -> (tempfile::TempDir, TestServer) {
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
        use modelsentry_common::config::{
            AlertsConfig, AuthConfig, DatabaseConfig, ProvidersConfig, SchedulerConfig,
            ServerConfig, VaultConfig,
        };
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

    fn probe_body() -> serde_json::Value {
        json!({
            "name": "test-probe",
            "provider": { "kind": "anthropic" },
            "model": "claude-3-5-haiku-20241022",
            "prompts": [{ "id": uuid::Uuid::new_v4(), "text": "ping?", "expected_contains": null, "expected_not_contains": null }],
            "schedule": { "kind": "every_minutes", "minutes": 60 }
        })
    }

    #[tokio::test]
    async fn list_probes_returns_empty_initially() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let resp = server.get("/probes").await;
        resp.assert_status_ok();
        resp.assert_json(&json!([]));
    }

    #[tokio::test]
    async fn create_probe_returns_201_with_probe_body() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let resp = server.post("/probes").json(&probe_body()).await;
        resp.assert_status(StatusCode::CREATED);
        let body: serde_json::Value = resp.json();
        assert_eq!(body["name"], "test-probe");
        assert!(body["id"].is_string());
    }

    #[tokio::test]
    async fn get_probe_returns_404_for_unknown_id() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let fake_id = uuid::Uuid::new_v4();
        let resp = server.get(&format!("/probes/{fake_id}")).await;
        resp.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_probe_returns_204() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(Arc::clone(&store));

        // create
        let create = server.post("/probes").json(&probe_body()).await;
        create.assert_status(StatusCode::CREATED);
        let id = create.json::<serde_json::Value>()["id"]
            .as_str()
            .unwrap()
            .to_string();

        // delete
        let del = server.delete(&format!("/probes/{id}")).await;
        del.assert_status(StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn create_probe_with_missing_name_returns_422() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let bad = json!({
            "provider": { "kind": "anthropic" },
            "model": "m",
            "prompts": [],
            "schedule": { "kind": "every_minutes", "minutes": 60 }
        });
        let resp = server.post("/probes").json(&bad).await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn create_probe_with_empty_model_returns_422() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let bad = json!({
            "name": "test",
            "provider": { "kind": "anthropic" },
            "model": "  ",
            "prompts": [{ "id": uuid::Uuid::new_v4(), "text": "ping", "expected_contains": null, "expected_not_contains": null }],
            "schedule": { "kind": "every_minutes", "minutes": 60 }
        });
        let resp = server.post("/probes").json(&bad).await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn create_probe_with_empty_prompts_returns_422() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let bad = json!({
            "name": "test",
            "provider": { "kind": "anthropic" },
            "model": "m",
            "prompts": [],
            "schedule": { "kind": "every_minutes", "minutes": 60 }
        });
        let resp = server.post("/probes").json(&bad).await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn create_probe_with_blank_prompt_text_returns_422() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(store);
        let bad = json!({
            "name": "test",
            "provider": { "kind": "anthropic" },
            "model": "m",
            "prompts": [{ "id": uuid::Uuid::new_v4(), "text": "  ", "expected_contains": null, "expected_not_contains": null }],
            "schedule": { "kind": "every_minutes", "minutes": 60 }
        });
        let resp = server.post("/probes").json(&bad).await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
}
