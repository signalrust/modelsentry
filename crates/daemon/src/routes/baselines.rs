//! Baseline endpoints.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use chrono::Utc;
use modelsentry_common::{
    error::ModelSentryError,
    models::BaselineSnapshot,
    types::{BaselineId, ProbeId},
};

use crate::{routes::AppError, server::AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/probes/{id}/baselines",
            get(list_baselines).post(capture_baseline),
        )
        .route("/probes/{id}/baselines/latest", get(get_latest_baseline))
        .route("/baselines/{id}", delete(delete_baseline))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_baselines(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<BaselineSnapshot>>, AppError> {
    let probe_id = parse_probe_id(&id)?;
    let baselines = state.store.baselines().list_for_probe(&probe_id)?;
    Ok(Json(baselines))
}

/// Capture a new baseline from the most recent run that has embeddings.
async fn capture_baseline(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<BaselineSnapshot>), AppError> {
    let probe_id = parse_probe_id(&id)?;

    let run = state
        .store
        .runs()
        .list_for_probe(&probe_id, 1)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError(ModelSentryError::Config {
                message: "no runs found for probe — run the probe first".to_string(),
            })
        })?;

    let valid_embeddings: Vec<&Vec<f32>> =
        run.embeddings.iter().filter(|e| !e.is_empty()).collect();

    if valid_embeddings.is_empty() {
        return Err(AppError(ModelSentryError::Config {
            message: "most recent run has no embeddings — cannot capture baseline".to_string(),
        }));
    }

    // Compute centroid = component-wise mean.
    let dim = valid_embeddings[0].len();
    #[allow(clippy::cast_precision_loss)]
    let n = valid_embeddings.len() as f32;
    let mut centroid = vec![0.0_f32; dim];
    for emb in &valid_embeddings {
        for (c, v) in centroid.iter_mut().zip(emb.iter()) {
            *c += v;
        }
    }
    for c in &mut centroid {
        *c /= n;
    }

    // Compute variance of L2 norms.
    let norms: Vec<f32> = valid_embeddings
        .iter()
        .map(|e| e.iter().map(|x| x * x).sum::<f32>().sqrt())
        .collect();
    let mean_norm = norms.iter().sum::<f32>() / n;
    let variance = norms
        .iter()
        .map(|norm| (norm - mean_norm).powi(2))
        .sum::<f32>()
        / n;

    let baseline = BaselineSnapshot {
        id: BaselineId::new(),
        probe_id: probe_id.clone(),
        captured_at: Utc::now(),
        embedding_centroid: centroid,
        embedding_variance: variance,
        output_tokens: run
            .completions
            .iter()
            .map(|c| c.split_whitespace().map(str::to_lowercase).collect())
            .collect(),
        run_id: run.id.clone(),
    };

    state.store.baselines().insert(&baseline)?;
    Ok((StatusCode::CREATED, Json(baseline)))
}

async fn get_latest_baseline(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BaselineSnapshot>, AppError> {
    let probe_id = parse_probe_id(&id)?;
    state
        .store
        .baselines()
        .get_latest_for_probe(&probe_id)?
        .map(Json)
        .ok_or_else(|| AppError(ModelSentryError::BaselineNotFound { id: id.clone() }))
}

async fn delete_baseline(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let baseline_id = parse_baseline_id(&id)?;
    let found = state.store.baselines().delete(&baseline_id)?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError(ModelSentryError::BaselineNotFound {
            id: id.clone(),
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

fn parse_baseline_id(s: &str) -> Result<BaselineId, AppError> {
    uuid::Uuid::parse_str(s)
        .map(BaselineId::from_uuid)
        .map_err(|_| AppError(ModelSentryError::BaselineNotFound { id: s.to_string() }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum_test::TestServer;
    use chrono::Utc;
    use modelsentry_common::{
        config::{AlertsConfig, AuthConfig, DatabaseConfig, ProvidersConfig, SchedulerConfig, ServerConfig, VaultConfig},
        models::{Probe, ProbeSchedule, ProviderKind},
    };
    use std::sync::Arc;

    use crate::scheduler::new_registry;
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
            providers: Arc::new(new_registry()),
            calculator: Arc::new(DriftCalculator::new(0.5, 0.5).unwrap()),
            alert_engine: Arc::new(AlertEngine::default()),
            config: Arc::new(modelsentry_common::config::AppConfig {
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
            }),
        };
        (vault_dir, TestServer::new(crate::routes::router(state)))
    }

    fn make_probe(store: &Arc<AppStore>) -> Probe {
        let probe = Probe {
            id: modelsentry_common::types::ProbeId::new(),
            name: "p".to_string(),
            provider: ProviderKind::Anthropic,
            model: "m".to_string(),
            prompts: vec![],
            schedule: ProbeSchedule::EveryMinutes { minutes: 60 },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.probes().insert(&probe).unwrap();
        probe
    }

    /// Capturing a baseline when there are no runs must return 422.
    #[tokio::test]
    async fn capture_baseline_requires_at_least_one_run() {
        let (_dir, store) = open_store();
        let probe = make_probe(&store);
        let (_vault_dir, server) = test_app(Arc::clone(&store));
        let resp = server
            .post(&format!("/probes/{}/baselines", probe.id))
            .await;
        resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
}
