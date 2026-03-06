//! Alert rule and event endpoints.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use modelsentry_common::{
    error::ModelSentryError,
    models::{AlertChannel, AlertEvent, AlertRule},
    types::{AlertRuleId, ProbeId},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{routes::AppError, server::AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/probes/{id}/alerts", get(list_rules).post(create_rule))
        .route("/alerts/{id}", delete(delete_rule))
        .route("/events", get(list_events))
        .route("/events/{id}/acknowledge", post(acknowledge_event))
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    pub kl_threshold: f32,
    pub cosine_threshold: f32,
    pub channels: Vec<AlertChannel>,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_active() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    #[serde(default = "default_event_limit")]
    pub limit: usize,
}

fn default_event_limit() -> usize {
    50
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_rules(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<AlertRule>>, AppError> {
    let probe_id = parse_probe_id(&id)?;
    let rules = state.store.alerts().get_rules_for_probe(&probe_id)?;
    Ok(Json(rules))
}

async fn create_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateRuleRequest>,
) -> Result<(StatusCode, Json<AlertRule>), AppError> {
    let probe_id = parse_probe_id(&id)?;
    let rule = AlertRule {
        id: AlertRuleId::new(),
        probe_id,
        kl_threshold: body.kl_threshold,
        cosine_threshold: body.cosine_threshold,
        channels: body.channels,
        active: body.active,
    };
    state.store.alerts().insert_rule(&rule)?;
    Ok((StatusCode::CREATED, Json(rule)))
}

async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let rule_id = parse_rule_id(&id)?;
    let found = state.store.alerts().delete_rule(&rule_id)?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError(ModelSentryError::Config {
            message: format!("alert rule not found: {id}"),
        }))
    }
}

async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<ListEventsQuery>,
) -> Result<Json<Vec<AlertEvent>>, AppError> {
    let events = state.store.alerts().list_events(query.limit)?;
    Ok(Json(events))
}

async fn acknowledge_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let event_id = parse_event_id(&id)?;
    let found = state.store.alerts().acknowledge_event(&event_id)?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError(ModelSentryError::Config {
            message: format!("alert event not found: {id}"),
        }))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_probe_id(s: &str) -> Result<ProbeId, AppError> {
    uuid::Uuid::parse_str(s)
        .map(ProbeId::from_uuid)
        .map_err(|_| AppError(ModelSentryError::ProbeNotFound { id: s.to_string() }))
}

fn parse_rule_id(s: &str) -> Result<AlertRuleId, AppError> {
    uuid::Uuid::parse_str(s)
        .map(AlertRuleId::from_uuid)
        .map_err(|_| {
            AppError(ModelSentryError::Config {
                message: format!("invalid alert rule id: {s}"),
            })
        })
}

fn parse_event_id(s: &str) -> Result<Uuid, AppError> {
    uuid::Uuid::parse_str(s).map_err(|_| {
        AppError(ModelSentryError::Config {
            message: format!("invalid event id: {s}"),
        })
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum_test::TestServer;
    use modelsentry_common::config::{
        AlertsConfig, DatabaseConfig, SchedulerConfig, ServerConfig, VaultConfig,
    };
    use serde_json::json;
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

    #[tokio::test]
    async fn create_alert_rule_with_webhook_channel() {
        let (_dir, store) = open_store();
        let probe_id = ProbeId::new();
        let (_vault_dir, server) = test_app(Arc::clone(&store));

        let body = json!({
            "kl_threshold": 0.3,
            "cosine_threshold": 0.2,
            "channels": [{ "kind": "webhook", "url": "https://example.com/hook" }],
            "active": true
        });
        let resp = server
            .post(&format!("/probes/{probe_id}/alerts"))
            .json(&body)
            .await;
        resp.assert_status(StatusCode::CREATED);
        let rule: serde_json::Value = resp.json();
        assert!(rule["id"].is_string());
        assert_eq!(rule["kl_threshold"], 0.3);
    }

    #[tokio::test]
    async fn acknowledge_event_returns_422_for_unknown() {
        let (_dir, store) = open_store();
        let (_vault_dir, server) = test_app(Arc::clone(&store));
        let fake_id = uuid::Uuid::new_v4();
        let resp = server.post(&format!("/events/{fake_id}/acknowledge")).await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
}
