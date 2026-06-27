//! Integration tests: end-to-end probe lifecycle using a mock LLM provider.
//!
//! No external services required — the mock provider is a local stub struct.
//! The persistence layer uses a temporary `redb` database.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use modelsentry_common::{
    constants::defaults,
    error::Result as CoreResult,
    models::{
        BaselineSnapshot, DriftLevel, Probe, ProbePrompt, ProbeSchedule, ProviderSpec, RunStatus,
    },
    types::{BaselineId, ProbeId, RunId},
};
use modelsentry_core::{
    drift::{Embedding, assessment::AssessmentConfig, calculator::DriftCalculator},
    probe_runner::ProbeRunner,
    provider::LlmProvider,
};
use modelsentry_store::AppStore;
use tempfile::TempDir;

// ── Stub provider ─────────────────────────────────────────────────────────────

/// A simple stub `LlmProvider` that always returns the same embedding and
/// completion for every call.
struct StubProvider {
    embedding: Vec<f32>,
    completion: String,
    fail_embed: bool,
}

impl StubProvider {
    fn new(embedding: Vec<f32>, completion: impl Into<String>) -> Self {
        Self {
            embedding,
            completion: completion.into(),
            fail_embed: false,
        }
    }

    fn flaky(embedding: Vec<f32>, completion: impl Into<String>) -> Self {
        Self {
            embedding,
            completion: completion.into(),
            fail_embed: true,
        }
    }
}

#[async_trait]
impl LlmProvider for StubProvider {
    async fn embed(&self, _texts: &[String]) -> CoreResult<Vec<Embedding>> {
        if self.fail_embed {
            return Err(modelsentry_common::error::ModelSentryError::Provider {
                message: "simulated timeout".into(),
            });
        }
        Ok(vec![Embedding::new(self.embedding.clone())?])
    }

    async fn complete(&self, _prompt: &str) -> CoreResult<String> {
        Ok(self.completion.clone())
    }

    fn provider_name(&self) -> &'static str {
        "stub"
    }

    fn embedding_dim(&self) -> usize {
        self.embedding.len()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn open_store() -> (TempDir, AppStore) {
    let dir = TempDir::new().unwrap();
    let store = AppStore::open(&dir.path().join("test.db")).unwrap();
    (dir, store)
}

fn make_probe() -> Probe {
    Probe {
        id: ProbeId::new(),
        name: "test-probe".into(),
        provider: ProviderSpec::OpenAi {
            model: defaults::openai::MODEL.into(),
        },
        prompts: vec![
            ProbePrompt {
                id: uuid::Uuid::new_v4(),
                text: "Explain gravity.".into(),
                expected_contains: None,
                expected_not_contains: None,
            },
            ProbePrompt {
                id: uuid::Uuid::new_v4(),
                text: "What is entropy?".into(),
                expected_contains: None,
                expected_not_contains: None,
            },
        ],
        schedule: ProbeSchedule::EveryMinutes { minutes: 60 },
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// Build a rich baseline: for each of `n_prompts` prompts, a cloud of `k`
/// jittered samples around `center`. (Real capture aggregates this many *runs*;
/// here we synthesize the cloud so the calibrated test has statistical power.)
fn jittered_baseline(
    probe_id: ProbeId,
    run_id: RunId,
    center: &[f32],
    n_prompts: usize,
    k: usize,
) -> BaselineSnapshot {
    let prompt_clouds: Vec<Vec<Vec<f32>>> = (0..n_prompts)
        .map(|_| {
            (0..k)
                .map(|i| {
                    #[allow(clippy::cast_precision_loss)]
                    let jitter = (i as f32 - k as f32 / 2.0) * 0.001;
                    center.iter().map(|c| c + jitter).collect()
                })
                .collect()
        })
        .collect();
    BaselineSnapshot {
        id: BaselineId::new(),
        probe_id,
        captured_at: Utc::now(),
        schema_version: modelsentry_common::models::BASELINE_SCHEMA_VERSION,
        embedding_model: "stub".to_string(),
        prompt_clouds,
        n_runs: k,
        run_id,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Full lifecycle: run probe → capture baseline → run again with different
/// embeddings → compute drift report → assert drift is detected.
#[tokio::test]
async fn full_lifecycle_create_probe_capture_baseline_detect_drift() {
    let (_dir, store) = open_store();
    let probe = make_probe();
    store.probes().insert(&probe).unwrap();

    // --- baseline run (stable embeddings) ---
    let baseline_runner = ProbeRunner::new(Arc::new(StubProvider::new(
        vec![1.0_f32, 0.0, 0.0],
        "Gravity is a fundamental force.",
    )));
    let baseline_run = baseline_runner.run(&probe, 4).await.unwrap();
    assert_eq!(baseline_run.status, RunStatus::Success);
    store.runs().insert(&baseline_run).unwrap();

    let baseline = jittered_baseline(
        probe.id.clone(),
        baseline_run.id.clone(),
        &[1.0_f32, 0.0, 0.0],
        probe.prompts.len(),
        60,
    );
    store.baselines().insert(&baseline).unwrap();

    // --- second run (shifted output embeddings — simulate drift) ---
    let drift_runner = ProbeRunner::new(Arc::new(StubProvider::new(
        vec![0.0_f32, 1.0, 0.0], // orthogonal to baseline
        "The quick brown fox jumped over many hurdles today.",
    )));
    let drift_run = drift_runner.run(&probe, 4).await.unwrap();
    assert_eq!(drift_run.status, RunStatus::Success);
    store.runs().insert(&drift_run).unwrap();

    // --- compute drift ---
    let calc = DriftCalculator::new(AssessmentConfig {
        target_fpr: 0.05,
        ..AssessmentConfig::default()
    });
    let report = calc.compute(&drift_run, &baseline).unwrap();

    assert!(
        report.combined_p_value < 0.05,
        "orthogonal output shift should be significant, p={}",
        report.combined_p_value
    );
    assert_ne!(
        report.drift_level,
        DriftLevel::None,
        "drift should be detected after output shift"
    );

    // Confirm persisted runs and baseline are retrievable
    let stored_runs = store.runs().list_for_probe(&probe.id, 10).unwrap();
    assert_eq!(stored_runs.len(), 2);
    let stored_baselines = store.baselines().list_for_probe(&probe.id).unwrap();
    assert_eq!(stored_baselines.len(), 1);
}

/// When the provider partially fails (embed fails for all prompts), the
/// `ProbeRunner` returns `PartialFailure` or `Failed` but does not panic.
#[tokio::test]
async fn probe_survives_provider_flake_and_retries() {
    let probe = make_probe();

    // Provider that always fails embed calls (simulates a flaky network)
    let runner = ProbeRunner::new(Arc::new(StubProvider::flaky(
        vec![1.0_f32, 0.0, 0.0],
        "some output",
    )));
    let run = runner.run(&probe, 2).await.unwrap();

    // All embed calls fail → status must not be Success
    assert!(
        matches!(run.status, RunStatus::PartialFailure | RunStatus::Failed),
        "expected PartialFailure or Failed when all embeds fail, got {:?}",
        run.status
    );
    // Completions should still be collected
    assert_eq!(run.completions.len(), probe.prompts.len());
}

/// Deleting a probe cascades to its associated runs and baselines.
#[tokio::test]
async fn delete_probe_also_deletes_associated_runs_and_baselines() {
    let (_dir, store) = open_store();
    let probe = make_probe();
    let probe_id = probe.id.clone();
    store.probes().insert(&probe).unwrap();

    let make_run = |id: RunId| modelsentry_common::models::ProbeRun {
        id,
        probe_id: probe_id.clone(),
        started_at: Utc::now(),
        finished_at: Utc::now(),
        embeddings: vec![vec![1.0, 0.0, 0.0]],
        completions: vec!["hello".into()],
        drift_report: None,
        status: RunStatus::Success,
    };
    store.runs().insert(&make_run(RunId::new())).unwrap();
    store.runs().insert(&make_run(RunId::new())).unwrap();

    let baseline = BaselineSnapshot {
        id: BaselineId::new(),
        probe_id: probe_id.clone(),
        captured_at: Utc::now(),
        schema_version: modelsentry_common::models::BASELINE_SCHEMA_VERSION,
        embedding_model: "stub".into(),
        prompt_clouds: vec![vec![vec![1.0, 0.0, 0.0]]],
        n_runs: 1,
        run_id: RunId::new(),
    };
    store.baselines().insert(&baseline).unwrap();

    // Confirm data is there before deletion
    assert_eq!(store.runs().list_for_probe(&probe_id, 10).unwrap().len(), 2);
    assert_eq!(
        store.baselines().list_for_probe(&probe_id).unwrap().len(),
        1
    );

    // Cascade delete
    let deleted = store.delete_probe_cascade(&probe_id).unwrap();
    assert!(
        deleted,
        "cascade delete should return true for existing probe"
    );

    // Probe is gone
    assert!(store.probes().get(&probe_id).unwrap().is_none());

    // Runs and baselines are also gone
    assert_eq!(
        store.runs().list_for_probe(&probe_id, 10).unwrap().len(),
        0,
        "runs for deleted probe should have been cascade-deleted"
    );
    assert_eq!(
        store.baselines().list_for_probe(&probe_id).unwrap().len(),
        0,
        "baselines for deleted probe should have been cascade-deleted"
    );
}
