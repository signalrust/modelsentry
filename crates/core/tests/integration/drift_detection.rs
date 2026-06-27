//! Integration tests: calibrated drift detection via the `DriftCalculator`.
//!
//! Verify that a run whose outputs sit inside the baseline clouds yields no
//! drift, and that a run whose output jumps away from its baseline cloud is
//! flagged with a significant (low) p-value.

use chrono::Utc;
use modelsentry_common::{
    models::{BASELINE_SCHEMA_VERSION, BaselineSnapshot, DriftLevel, ProbeRun, RunStatus},
    types::{BaselineId, ProbeId, RunId},
};
use modelsentry_core::drift::{assessment::AssessmentConfig, calculator::DriftCalculator};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn axis_point(dim: usize, x0: f32) -> Vec<f32> {
    let mut v = vec![0.0; dim];
    if let Some(f) = v.first_mut() {
        *f = x0;
    }
    v
}

#[allow(clippy::cast_precision_loss)]
fn cloud(count: usize, dim: usize, center: f32) -> Vec<Vec<f32>> {
    (0..count)
        .map(|i| {
            let off = (i as f32 - count as f32 / 2.0) * 0.02;
            axis_point(dim, center + off)
        })
        .collect()
}

fn make_run_and_baseline(
    run_embeddings: Vec<Vec<f32>>,
    prompt_clouds: Vec<Vec<Vec<f32>>>,
) -> (ProbeRun, BaselineSnapshot) {
    let probe_id = ProbeId::new();
    let run_id = RunId::new();

    let run = ProbeRun {
        id: run_id.clone(),
        probe_id: probe_id.clone(),
        started_at: Utc::now(),
        finished_at: Utc::now(),
        embeddings: run_embeddings,
        completions: vec!["answer".to_string()],
        drift_report: None,
        status: RunStatus::Success,
    };

    let baseline = BaselineSnapshot {
        id: BaselineId::new(),
        probe_id,
        captured_at: Utc::now(),
        schema_version: BASELINE_SCHEMA_VERSION,
        embedding_model: "test".to_string(),
        prompt_clouds,
        n_runs: 1,
        run_id,
    };

    (run, baseline)
}

fn calc(target_fpr: f32) -> DriftCalculator {
    DriftCalculator::new(AssessmentConfig {
        target_fpr,
        ..AssessmentConfig::default()
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[test]
fn no_drift_detected_when_model_stable() {
    // Two prompts; run outputs sit inside each baseline cloud.
    let (run, baseline) = make_run_and_baseline(
        vec![axis_point(4, 0.01), axis_point(4, 5.0)],
        vec![cloud(15, 4, 0.0), cloud(15, 4, 5.0)],
    );

    let report = calc(0.01).compute(&run, &baseline).unwrap();

    assert_eq!(report.drift_level, DriftLevel::None);
    assert!(
        report.combined_p_value > 0.05,
        "stable run should not be significant, p={}",
        report.combined_p_value
    );
}

#[test]
fn drift_detected_when_output_shifts_beyond_baseline() {
    // Prompt 1's output jumps far from its baseline cloud.
    let (run, baseline) = make_run_and_baseline(
        vec![axis_point(4, 0.0), axis_point(4, 25.0)],
        vec![cloud(60, 4, 0.0), cloud(60, 4, 10.0)],
    );

    let report = calc(0.05).compute(&run, &baseline).unwrap();

    assert!(
        report.combined_p_value < 0.05,
        "output shift should be significant, p={}",
        report.combined_p_value
    );
    assert_ne!(report.drift_level, DriftLevel::None);
    // The per-prompt breakdown should single out prompt #1 as the strongest.
    let strongest = report
        .per_prompt
        .iter()
        .min_by(|a, b| a.p_value.partial_cmp(&b.p_value).unwrap())
        .unwrap();
    assert_eq!(strongest.prompt_index, 1);
}

#[test]
fn single_sample_baseline_uses_pooled_fallback() {
    // One sample per prompt → pooled MMD/energy fallback.
    let (run, baseline) = make_run_and_baseline(
        vec![axis_point(3, 0.0), axis_point(3, 5.0)],
        vec![vec![axis_point(3, 0.0)], vec![axis_point(3, 5.0)]],
    );
    let report = calc(0.05).compute(&run, &baseline).unwrap();
    assert_eq!(
        report.method,
        modelsentry_common::constants::method::POOLED_TWO_SAMPLE
    );
}
