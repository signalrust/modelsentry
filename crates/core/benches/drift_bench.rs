// Criterion benchmark for the calibrated drift pipeline at representative
// embedding sizes.
//
// Run with: cargo bench -p modelsentry-core

// Benchmarks are not production code — unwrap is fine here.
#![allow(clippy::unwrap_used)]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use chrono::Utc;
use modelsentry_common::{
    models::{BaselineSnapshot, ProbeRun, RunStatus},
    types::{BaselineId, ProbeId, RunId},
};
use modelsentry_core::drift::{assessment::AssessmentConfig, calculator::DriftCalculator};

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn make_run_and_baseline(dim: usize) -> (ProbeRun, BaselineSnapshot) {
    let probe_id = ProbeId::new();
    let run_id = RunId::new();

    let emb: Vec<f32> = {
        let mut v = vec![0.0_f32; dim];
        v[0] = 1.0;
        v
    };

    let run = ProbeRun {
        id: run_id.clone(),
        probe_id: probe_id.clone(),
        started_at: Utc::now(),
        finished_at: Utc::now(),
        // 5 prompts, 3 samples each (the default multi-sample path).
        embeddings: vec![vec![emb.clone(); 3]; 5],
        completions: vec!["hello world foo bar baz".to_owned(); 5],
        drift_report: None,
        status: RunStatus::Success,
    };

    // 5 prompts, each with a baseline cloud of 20 jittered samples.
    let prompt_clouds: Vec<Vec<Vec<f32>>> = (0..5)
        .map(|_| {
            (0..20)
                .map(|i| {
                    let mut v = emb.clone();
                    if let Some(first) = v.first_mut() {
                        #[allow(clippy::cast_precision_loss)]
                        {
                            *first += (i as f32) * 0.0005;
                        }
                    }
                    v
                })
                .collect()
        })
        .collect();

    let baseline = BaselineSnapshot {
        id: BaselineId::new(),
        probe_id,
        captured_at: Utc::now(),
        schema_version: modelsentry_common::models::BASELINE_SCHEMA_VERSION,
        embedding_model: "bench".to_owned(),
        prompt_clouds,
        n_runs: 20,
        run_id,
    };

    (run, baseline)
}

// ── Benchmark groups ──────────────────────────────────────────────────────────

fn bench_drift_calculator_compute(c: &mut Criterion) {
    let mut group = c.benchmark_group("drift_calculator_compute");
    let calc = DriftCalculator::new(AssessmentConfig::default());

    for &dim in &[100_usize, 500, 1536] {
        group.bench_with_input(BenchmarkId::from_parameter(dim), &dim, |b, &dim| {
            let (run, baseline) = make_run_and_baseline(dim);
            b.iter(|| calc.compute(&run, &baseline).unwrap());
        });
    }

    group.finish();
}

criterion_group!(benches, bench_drift_calculator_compute);
criterion_main!(benches);
