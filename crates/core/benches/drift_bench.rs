// Criterion benchmarks for the three drift algorithms at representative embedding sizes.
//
// Run with: cargo bench -p modelsentry-core

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use modelsentry_core::drift::{
    Embedding,
    calculator::DriftCalculator,
    cosine::cosine_distance,
    entropy::entropy_delta,
    kl::gaussian_kl,
};
use modelsentry_common::{
    models::{BaselineSnapshot, ProbeRun, RunStatus},
    types::{BaselineId, ProbeId, RunId},
};
use chrono::Utc;

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn unit_embedding(dim: usize) -> Embedding {
    let mut v = vec![0.0_f32; dim];
    v[0] = 1.0;
    Embedding::new(v).unwrap()
}

#[allow(dead_code)]
fn embeddings(dim: usize, count: usize) -> Vec<Embedding> {
    (0..count).map(|_| unit_embedding(dim)).collect()
}

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
        embeddings: vec![emb.clone(); 5],
        completions: vec!["hello world foo bar baz".to_owned(); 5],
        drift_report: None,
        status: RunStatus::Success,
    };

    let baseline = BaselineSnapshot {
        id: BaselineId::new(),
        probe_id,
        captured_at: Utc::now(),
        embedding_centroid: emb,
        embedding_variance: 0.01,
        output_tokens: vec![
            vec!["hello".into(), "world".into(), "foo".into()],
            vec!["hello".into(), "world".into(), "foo".into()],
        ],
        run_id,
    };

    (run, baseline)
}

// ── Benchmark groups ──────────────────────────────────────────────────────────

fn bench_kl_divergence(c: &mut Criterion) {
    let mut group = c.benchmark_group("kl_divergence");

    for &n in &[100_usize, 500, 1536] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            // Gaussian KL is O(1) but we benchmark with realistic mu/sigma values
            // that would result from n-dimensional embeddings.
            let mu1 = 1.0_f32;
            let sigma1 = 0.1_f32;
            let mu2 = 1.0_f32 + (n as f32).sqrt() * 0.001;
            let sigma2 = 0.1_f32;
            b.iter(|| {
                gaussian_kl(mu1, sigma1, mu2, sigma2).unwrap()
            });
        });
    }

    group.finish();
}

fn bench_cosine_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("cosine_distance");

    for &n in &[100_usize, 500, 1536] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let a = unit_embedding(n);
            // Slightly rotated vector
            let mut raw = vec![0.0_f32; n];
            raw[0] = 0.9;
            if n > 1 { raw[1] = 0.1; }
            let b_emb = Embedding::new(raw).unwrap();
            b.iter(|| cosine_distance(&a, &b_emb).unwrap());
        });
    }

    group.finish();
}

fn bench_output_entropy(c: &mut Criterion) {
    let mut group = c.benchmark_group("output_entropy");

    for &n in &[10_usize, 50, 100] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            // n completions of 20 tokens each
            let completions: Vec<Vec<String>> = (0..n)
                .map(|i| {
                    (0..20).map(|j| format!("tok_{}", (i + j) % 15)).collect()
                })
                .collect();
            let baseline_tokens: Vec<Vec<String>> = completions.clone();
            b.iter(|| entropy_delta(&completions, &baseline_tokens).unwrap());
        });
    }

    group.finish();
}

fn bench_drift_calculator_compute(c: &mut Criterion) {
    let mut group = c.benchmark_group("drift_calculator_compute");
    let calc = DriftCalculator::new(0.5, 0.3).unwrap();

    for &dim in &[100_usize, 500, 1536] {
        group.bench_with_input(BenchmarkId::from_parameter(dim), &dim, |b, &dim| {
            let (run, baseline) = make_run_and_baseline(dim);
            b.iter(|| calc.compute(&run, &baseline).unwrap());
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_kl_divergence,
    bench_cosine_distance,
    bench_output_entropy,
    bench_drift_calculator_compute,
);
criterion_main!(benches);
