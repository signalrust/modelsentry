//! Integration tests: drift detection logic using the `DriftCalculator`.
//!
//! Tests verify that stable models produce no drift and that shifted embeddings
//! are correctly classified as drifted.

use chrono::Utc;
use modelsentry_common::{
    models::{BaselineSnapshot, DriftLevel, ProbeRun, RunStatus},
    types::{BaselineId, ProbeId, RunId},
};
use modelsentry_core::drift::calculator::DriftCalculator;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_run_and_baseline(
    run_embeddings: Vec<Vec<f32>>,
    baseline_centroid: Vec<f32>,
    baseline_variance: f32,
    run_completions: Vec<&str>,
    baseline_tokens: Vec<Vec<&str>>,
) -> (ProbeRun, BaselineSnapshot) {
    let probe_id = ProbeId::new();
    let run_id = RunId::new();

    let run = ProbeRun {
        id: run_id.clone(),
        probe_id: probe_id.clone(),
        started_at: Utc::now(),
        finished_at: Utc::now(),
        embeddings: run_embeddings,
        completions: run_completions.into_iter().map(String::from).collect(),
        drift_report: None,
        status: RunStatus::Success,
    };

    let baseline = BaselineSnapshot {
        id: BaselineId::new(),
        probe_id,
        captured_at: Utc::now(),
        embedding_centroid: baseline_centroid,
        embedding_variance: baseline_variance,
        output_tokens: baseline_tokens
            .into_iter()
            .map(|ts| ts.into_iter().map(String::from).collect())
            .collect(),
        run_id,
    };

    (run, baseline)
}

fn calc() -> DriftCalculator {
    // threshold: kl=0.1, cosine=0.15
    DriftCalculator::new(0.1, 0.15).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// When the model is completely stable (identical embeddings and completions),
/// the drift calculator must produce `DriftLevel::None` with near-zero metrics.
#[test]
fn no_drift_detected_when_model_stable() {
    let emb = vec![0.6_f32, 0.8, 0.0]; // unit-ish vector

    let (run, baseline) = make_run_and_baseline(
        vec![emb.clone(), emb.clone()],
        emb, // same centroid
        0.0, // zero variance
        vec!["hello world", "hello world"],
        vec![vec!["hello", "world"], vec!["hello", "world"]],
    );

    let report = calc().compute(&run, &baseline).unwrap();

    assert_eq!(
        report.drift_level,
        DriftLevel::None,
        "identical run and baseline should have DriftLevel::None"
    );
    assert!(
        report.cosine_distance < 1e-4,
        "cosine distance should be ~0 for identical embeddings, got {}",
        report.cosine_distance
    );
    assert!(
        report.kl_divergence.abs() < 0.1,
        "KL divergence should be below threshold for stable model, got {}",
        report.kl_divergence
    );
}

/// When embedding centroids shift significantly (orthogonal vectors), the
/// calculator must detect drift at `Medium` level or higher.
#[test]
fn drift_detected_when_embedding_centroid_shifts_beyond_threshold() {
    // Baseline: [1, 0, 0] — run: [0, 1, 0] — cosine distance = 0.5
    let (run, baseline) = make_run_and_baseline(
        vec![vec![0.0_f32, 1.0, 0.0], vec![0.0, 1.0, 0.0]],
        vec![1.0_f32, 0.0, 0.0], // orthogonal to run
        0.0,
        vec![
            "the quick brown fox jumped over",
            "a completely different sentence here",
        ],
        vec![vec!["hello"], vec!["hello"]],
    );

    let report = calc().compute(&run, &baseline).unwrap();

    assert!(
        report.cosine_distance > 0.4,
        "orthogonal shift should produce cosine distance > 0.4, got {}",
        report.cosine_distance
    );
    assert!(
        matches!(
            report.drift_level,
            DriftLevel::Medium | DriftLevel::High | DriftLevel::Critical
        ),
        "expected Medium+ drift level for large cosine shift, got {:?}",
        report.drift_level
    );
}

/// KL divergence increases when the embedding norm distribution shifts to a
/// much larger mean — validates the Gaussian KL formula path.
#[test]
fn kl_divergence_increases_with_norm_shift() {
    // Baseline centroid norm ≈ 1.0; run embeddings have norm ≈ 5.0
    let (run, baseline) = make_run_and_baseline(
        vec![
            vec![5.0_f32, 0.0, 0.0],
            vec![5.0, 0.0, 0.0],
            vec![5.0, 0.0, 0.0],
        ],
        vec![1.0_f32, 0.0, 0.0],
        0.0,
        vec!["a", "b", "c"],
        vec![vec!["a"], vec!["b"], vec!["c"]],
    );

    let report = calc().compute(&run, &baseline).unwrap();
    assert!(
        report.kl_divergence > 0.0,
        "large norm shift should produce positive KL divergence"
    );
    assert!(
        matches!(
            report.drift_level,
            DriftLevel::Low | DriftLevel::Medium | DriftLevel::High | DriftLevel::Critical
        ),
        "norm-shifted run should not be None drift, got {:?}",
        report.drift_level
    );
}
