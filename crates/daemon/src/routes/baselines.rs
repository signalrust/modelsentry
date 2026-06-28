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
    models::{BASELINE_SCHEMA_VERSION, BaselineSnapshot},
    types::{BaselineId, ProbeId, RunId},
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

/// Capture a new baseline by aggregating the most recent successful runs into
/// per-prompt output-embedding clouds (schema v2).
///
/// More runs ⇒ richer clouds ⇒ more statistical power for the conformal drift
/// test (see the drift-detection methodology). The number of runs folded in is
/// `[alerts] baseline_capture_runs`. Only runs whose embedding dimension matches
/// the newest run are aggregated, so a mid-stream embedding-model change never
/// mixes incompatible vectors into one cloud.
async fn capture_baseline(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<BaselineSnapshot>), AppError> {
    let probe_id = parse_probe_id(&id)?;

    let runs = state
        .store
        .runs()
        .list_for_probe(&probe_id, state.config.alerts.baseline_capture_runs)?;

    // Newest-first metadata; fetch each run's embeddings (kept in a separate
    // table) and keep only runs that produced at least one output embedding.
    // `(run_id, embeddings)`, newest first.
    let mut usable: Vec<(RunId, Vec<Vec<Vec<f32>>>)> = Vec::new();
    for run in &runs {
        let embeddings = state.store.runs().embeddings(&run.id)?.unwrap_or_default();
        if embeddings
            .iter()
            .any(|samples| samples.iter().any(|e| !e.is_empty()))
        {
            usable.push((run.id.clone(), embeddings));
        }
    }

    let (newest_id, newest_embeddings) = usable.first().ok_or_else(|| {
        AppError(ModelSentryError::Config {
            message: "no runs with embeddings found for probe — run the probe first".to_string(),
        })
    })?;
    let newest_id = newest_id.clone();

    // Pin the cloud to the newest run's embedding dimension; drop older runs
    // captured under a different embedding model (different dimension).
    let dim = embedding_dim(newest_embeddings);
    usable.retain(|(_, embeddings)| embedding_dim(embeddings) == dim);

    // Build per-prompt clouds: prompt_clouds[i] gathers every run's sample
    // embeddings for prompt i (skipping samples/prompts that failed).
    let n_prompts = usable
        .iter()
        .map(|(_, embeddings)| embeddings.len())
        .max()
        .unwrap_or(0);
    let mut prompt_clouds: Vec<Vec<Vec<f32>>> = vec![Vec::new(); n_prompts];
    for (_, embeddings) in &usable {
        for (cloud, samples) in prompt_clouds.iter_mut().zip(embeddings.iter()) {
            for sample in samples {
                if !sample.is_empty() {
                    cloud.push(sample.clone());
                }
            }
        }
    }

    let baseline = BaselineSnapshot {
        id: BaselineId::new(),
        probe_id: probe_id.clone(),
        captured_at: Utc::now(),
        schema_version: BASELINE_SCHEMA_VERSION,
        embedding_model: state.config.providers.openai.embedding_model.clone(),
        prompt_clouds,
        n_runs: usable.len(),
        run_id: newest_id,
    };

    state.store.baselines().insert(&baseline)?;
    Ok((StatusCode::CREATED, Json(baseline)))
}

/// Embedding dimensionality of a run's embeddings (length of the first non-empty
/// sample), or 0 if there are none.
fn embedding_dim(embeddings: &[Vec<Vec<f32>>]) -> usize {
    embeddings
        .iter()
        .flat_map(|samples| samples.iter())
        .find(|e| !e.is_empty())
        .map_or(0, Vec::len)
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
        config::{
            AlertsConfig, AuthConfig, DatabaseConfig, ProvidersConfig, SchedulerConfig,
            ServerConfig, VaultConfig,
        },
        constants::defaults,
        models::{Probe, ProbeSchedule, ProviderSpec},
    };
    use std::sync::Arc;

    use crate::server::AppState;
    use modelsentry_core::{
        alert::AlertEngine,
        drift::{assessment::AssessmentConfig, calculator::DriftCalculator},
    };
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
            calculator: Arc::new(DriftCalculator::new(AssessmentConfig::default())),
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
                    max_concurrent_runs: 8,
                },
                alerts: AlertsConfig::default(),
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
            provider: ProviderSpec::Anthropic {
                model: defaults::anthropic::MODEL.to_string(),
            },
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
