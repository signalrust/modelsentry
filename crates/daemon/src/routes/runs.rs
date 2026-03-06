//! Run history endpoints.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use modelsentry_common::{
    error::ModelSentryError,
    models::ProbeRun,
    types::{ProbeId, RunId},
};
use serde::Deserialize;

use crate::{routes::AppError, server::AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/probes/{id}/runs", get(list_runs))
        .route("/runs/{id}", get(get_run))
}

// ---------------------------------------------------------------------------
// Request params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ListRunsQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ListRunsQuery>,
) -> Result<Json<Vec<ProbeRun>>, AppError> {
    let probe_id = parse_probe_id(&id)?;
    let runs = state.store.runs().list_for_probe(&probe_id, query.limit)?;
    Ok(Json(runs))
}

async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProbeRun>, AppError> {
    let run_id = parse_run_id(&id)?;
    state.store.runs().get(&run_id)?.map(Json).ok_or_else(|| {
        AppError(ModelSentryError::Config {
            message: format!("run not found: {id}"),
        })
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_probe_id(s: &str) -> Result<ProbeId, AppError> {
    uuid::Uuid::parse_str(s)
        .map(ProbeId::from_uuid)
        .map_err(|_| AppError(ModelSentryError::ProbeNotFound { id: s.to_string() }))
}

fn parse_run_id(s: &str) -> Result<RunId, AppError> {
    uuid::Uuid::parse_str(s).map(RunId::from_uuid).map_err(|_| {
        AppError(ModelSentryError::Config {
            message: format!("invalid run id: {s}"),
        })
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use chrono::Utc;
    use modelsentry_common::{
        config::{AlertsConfig, DatabaseConfig, SchedulerConfig, ServerConfig, VaultConfig},
        models::{ProbeRun, RunStatus},
        types::RunId,
    };
    use std::sync::Arc;

    use crate::scheduler::ProviderRegistry;
    use crate::server::AppState;
    use modelsentry_core::{alert::AlertEngine, drift::calculator::DriftCalculator};
    use modelsentry_store::AppStore;

    fn open_store() -> (tempfile::TempDir, Arc<AppStore>) {
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
            providers: Arc::new(ProviderRegistry::new()),
            calculator: Arc::new(DriftCalculator::new(0.5, 0.5).unwrap()),
            alert_engine: Arc::new(AlertEngine::default()),
            config: Arc::new(modelsentry_common::config::AppConfig {
                server: ServerConfig {
                    host: "127.0.0.1".to_string(),
                    port: 7740,
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
            }),
        };
        (vault_dir, TestServer::new(crate::routes::router(state)))
    }

    fn make_run(probe_id: &ProbeId) -> ProbeRun {
        let now = Utc::now();
        ProbeRun {
            id: RunId::new(),
            probe_id: probe_id.clone(),
            started_at: now,
            finished_at: now,
            embeddings: vec![],
            completions: vec!["ok".to_string()],
            drift_report: None,
            status: RunStatus::Success,
        }
    }

    #[tokio::test]
    async fn list_runs_returns_empty_for_new_probe() {
        let (_dir, store) = open_store();
        let probe_id = ProbeId::new();
        let (_vault_dir, server) = test_app(Arc::clone(&store));
        let resp = server.get(&format!("/probes/{probe_id}/runs")).await;
        resp.assert_status_ok();
        let body: serde_json::Value = resp.json();
        assert_eq!(body, serde_json::json!([]));
    }

    #[tokio::test]
    async fn list_runs_respects_limit_query_param() {
        let (_dir, store) = open_store();
        let probe_id = ProbeId::new();
        for _ in 0..5 {
            store.runs().insert(&make_run(&probe_id)).unwrap();
        }
        let (_vault_dir, server) = test_app(Arc::clone(&store));
        let resp = server
            .get(&format!("/probes/{probe_id}/runs?limit=2"))
            .await;
        resp.assert_status_ok();
        let body: Vec<serde_json::Value> = resp.json();
        assert_eq!(body.len(), 2);
    }

    #[tokio::test]
    async fn get_run_returns_422_for_unknown() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(Arc::clone(&store));
        let fake_id = uuid::Uuid::new_v4();
        let resp = server.get(&format!("/runs/{fake_id}")).await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
}
