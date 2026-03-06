//! High-level drift calculator: composes KL, cosine, and entropy into a
//! single [`DriftReport`].

use chrono::Utc;
use modelsentry_common::{
    error::{ModelSentryError, Result},
    models::{BaselineSnapshot, DriftLevel, DriftReport, ProbeRun},
};

use super::{cosine, entropy, kl, Embedding};

/// Minimum standard deviation used when modelling embedding norm distributions
/// as Gaussians. Prevents division by zero when all embeddings have identical
/// norms (zero empirical variance).
const SIGMA_FLOOR: f32 = 0.1;

/// Composes KL divergence, cosine distance, and output entropy into a
/// [`DriftReport`] for a single probe run against its baseline.
pub struct DriftCalculator {
    kl_threshold: f32,
    cosine_threshold: f32,
}

impl DriftCalculator {
    /// Create a new calculator with the given alert thresholds.
    ///
    /// # Errors
    ///
    /// [`ModelSentryError::Config`] if either threshold is non-positive.
    pub fn new(kl_threshold: f32, cosine_threshold: f32) -> Result<Self> {
        if kl_threshold <= 0.0 || cosine_threshold <= 0.0 {
            return Err(ModelSentryError::Config {
                message: format!(
                    "thresholds must be positive; got kl={kl_threshold}, cosine={cosine_threshold}"
                ),
            });
        }
        Ok(Self {
            kl_threshold,
            cosine_threshold,
        })
    }

    /// Compute a full [`DriftReport`] comparing `run` to `baseline`.
    ///
    /// ### KL divergence
    /// Models the distribution of embedding L2 norms as a univariate Gaussian
    /// and applies the closed-form Gaussian KL formula.
    ///
    /// ### Cosine distance
    /// Distance between the run's centroid and the baseline centroid.
    ///
    /// ### Output entropy delta
    /// Absolute difference in mean Shannon entropy between run completions
    /// (whitespace-tokenized) and the baseline token lists.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::EmptyEmbedding`] if the run has no embeddings.
    /// - [`ModelSentryError::DimensionMismatch`] if run and baseline embedding
    ///   dimensions differ.
    /// - Propagates other errors from the underlying algorithms.
    pub fn compute(&self, run: &ProbeRun, baseline: &BaselineSnapshot) -> Result<DriftReport> {
        // --- build typed Embedding values from run ---
        if run.embeddings.is_empty() {
            return Err(ModelSentryError::EmptyEmbedding);
        }
        let run_embeddings: Vec<Embedding> = run
            .embeddings
            .iter()
            .map(|v| Embedding::new(v.clone()))
            .collect::<Result<Vec<_>>>()?;

        let baseline_embedding = Embedding::new(baseline.embedding_centroid.clone())?;

        // --- cosine distance: run centroid vs baseline centroid ---
        let run_centroid = Embedding::centroid(&run_embeddings)?;
        let cos_dist = cosine::cosine_distance(&run_centroid, &baseline_embedding)?;

        // --- KL divergence: model embedding norm distributions as Gaussians ---
        let run_norms: Vec<f32> = run_embeddings.iter().map(Embedding::l2_norm).collect();
        #[allow(clippy::cast_precision_loss)]
        let n = run_norms.len() as f32;
        let mu_run = run_norms.iter().sum::<f32>() / n;
        let variance_run =
            run_norms.iter().map(|x| (x - mu_run) * (x - mu_run)).sum::<f32>() / n;
        let sigma_run = variance_run.sqrt().max(SIGMA_FLOOR);

        let mu_baseline = baseline_embedding.l2_norm();
        let sigma_baseline = baseline.embedding_variance.sqrt().max(SIGMA_FLOOR);

        let kl = kl::gaussian_kl(mu_run, sigma_run, mu_baseline, sigma_baseline)?;

        // --- output entropy delta ---
        let run_tokens: Vec<Vec<String>> = run
            .completions
            .iter()
            .map(|c| entropy::tokenize(c))
            .collect();
        let ent_delta = entropy::entropy_delta(&run_tokens, &baseline.output_tokens)?;

        let level = self.classify_level(kl, cos_dist);

        Ok(DriftReport {
            run_id: run.id.clone(),
            baseline_id: baseline.id.clone(),
            kl_divergence: kl,
            cosine_distance: cos_dist,
            output_entropy_delta: ent_delta,
            drift_level: level,
            computed_at: Utc::now(),
        })
    }

    /// Map raw metric values to a [`DriftLevel`] using the configured thresholds.
    ///
    /// The worst of the two metric ratios determines the level:
    /// - `< 1×` threshold → `None`
    /// - `1×–2×` → `Low`
    /// - `2×–4×` → `Medium`
    /// - `4×–8×` → `High`
    /// - `≥ 8×`  → `Critical`
    fn classify_level(&self, kl: f32, cosine: f32) -> DriftLevel {
        if kl < self.kl_threshold && cosine < self.cosine_threshold {
            return DriftLevel::None;
        }
        let kl_ratio = kl / self.kl_threshold;
        let cosine_ratio = cosine / self.cosine_threshold;
        let max_ratio = kl_ratio.max(cosine_ratio);
        if max_ratio < 2.0 {
            DriftLevel::Low
        } else if max_ratio < 4.0 {
            DriftLevel::Medium
        } else if max_ratio < 8.0 {
            DriftLevel::High
        } else {
            DriftLevel::Critical
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelsentry_common::{
        models::{DriftLevel, RunStatus},
        types::{BaselineId, ProbeId, RunId},
    };

    const EPS: f32 = 1e-4_f32;

    /// Builds a run and baseline where both have identical unit-vector embeddings
    /// and identical completions — expect zero drift in all metrics.
    fn make_identical_run_and_baseline() -> (ProbeRun, BaselineSnapshot) {
        let probe_id = ProbeId::new();
        let run_id = RunId::new();
        let baseline_id = BaselineId::new();

        // Unit vector [1, 0, 0]; two copies so variance is still 0
        let emb = vec![1.0_f32, 0.0, 0.0];

        let run = ProbeRun {
            id: run_id.clone(),
            probe_id: probe_id.clone(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            embeddings: vec![emb.clone(), emb.clone()],
            completions: vec!["hello world".into(), "hello world".into()],
            drift_report: None,
            status: RunStatus::Success,
        };

        let baseline = BaselineSnapshot {
            id: baseline_id,
            probe_id,
            captured_at: Utc::now(),
            embedding_centroid: emb,
            embedding_variance: 0.0,
            output_tokens: vec![
                vec!["hello".into(), "world".into()],
                vec!["hello".into(), "world".into()],
            ],
            run_id,
        };

        (run, baseline)
    }

    /// Builds a run where embeddings have double the norm of the baseline
    /// centroid, triggering high KL divergence.
    fn make_drifted_run_and_baseline() -> (ProbeRun, BaselineSnapshot) {
        let probe_id = ProbeId::new();
        let run_id = RunId::new();
        let baseline_id = BaselineId::new();

        let run = ProbeRun {
            id: run_id.clone(),
            probe_id: probe_id.clone(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            // Double-magnitude, same direction as baseline → cosine=0, high KL
            embeddings: vec![vec![2.0_f32, 0.0, 0.0], vec![2.0, 0.0, 0.0]],
            completions: vec![
                "the quick brown fox jumped".into(),
                "the quick brown fox jumped".into(),
            ],
            drift_report: None,
            status: RunStatus::Success,
        };

        let baseline = BaselineSnapshot {
            id: baseline_id,
            probe_id,
            captured_at: Utc::now(),
            embedding_centroid: vec![1.0_f32, 0.0, 0.0],
            embedding_variance: 0.0,
            output_tokens: vec![
                vec!["hello".into()],
                vec!["hello".into()],
            ],
            run_id,
        };

        (run, baseline)
    }

    fn calculator() -> DriftCalculator {
        DriftCalculator::new(0.1, 0.15).unwrap()
    }

    #[test]
    fn identical_run_and_baseline_produces_none_drift() {
        let (run, baseline) = make_identical_run_and_baseline();
        let report = calculator().compute(&run, &baseline).unwrap();
        assert!(
            report.kl_divergence.abs() < EPS,
            "expected kl≈0, got {}",
            report.kl_divergence
        );
        assert!(
            report.cosine_distance.abs() < EPS,
            "expected cosine≈0, got {}",
            report.cosine_distance
        );
        assert_eq!(report.drift_level, DriftLevel::None);
    }

    #[test]
    fn high_kl_produces_high_or_critical_drift() {
        let (run, baseline) = make_drifted_run_and_baseline();
        let report = calculator().compute(&run, &baseline).unwrap();
        assert!(
            matches!(report.drift_level, DriftLevel::High | DriftLevel::Critical),
            "expected High or Critical, got {:?}",
            report.drift_level
        );
    }

    #[test]
    fn dimension_mismatch_returns_error() {
        let (mut run, baseline) = make_identical_run_and_baseline();
        // Swap run embeddings for dim-2 vectors while baseline centroid is dim-3
        run.embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        assert!(calculator().compute(&run, &baseline).is_err());
    }

    #[test]
    fn report_contains_correct_run_and_baseline_ids() {
        let (run, baseline) = make_identical_run_and_baseline();
        let report = calculator().compute(&run, &baseline).unwrap();
        assert_eq!(report.run_id, run.id);
        assert_eq!(report.baseline_id, baseline.id);
    }

    #[test]
    fn drift_level_thresholds_match_constructor_config() {
        // With tight thresholds, even small differences should register as drift
        let tight = DriftCalculator::new(0.001, 0.001).unwrap();
        let (run, baseline) = make_identical_run_and_baseline();
        // Identical run/baseline — still None regardless of thresholds
        let report = tight.compute(&run, &baseline).unwrap();
        assert_eq!(report.drift_level, DriftLevel::None);
    }

    #[test]
    fn new_rejects_non_positive_thresholds() {
        assert!(DriftCalculator::new(0.0, 0.1).is_err());
        assert!(DriftCalculator::new(0.1, 0.0).is_err());
        assert!(DriftCalculator::new(-1.0, 0.1).is_err());
    }

    // --- DriftLevel boundary tests (all four non-None buckets) ---
    // These call classify_level directly since it's private but accessible within
    // the same module's test block.

    #[test]
    fn classify_level_none_below_both_thresholds() {
        let calc = calculator();
        assert_eq!(calc.classify_level(0.05, 0.05), DriftLevel::None);
    }

    #[test]
    fn classify_level_low_at_1x_to_2x() {
        // kl = 1.5× threshold (0.15), cosine = 0 → max_ratio = 1.5 → Low
        let calc = calculator();
        assert_eq!(calc.classify_level(0.15, 0.0), DriftLevel::Low);
    }

    #[test]
    fn classify_level_medium_at_2x_to_4x() {
        // kl = 2.5× threshold (0.25) → Medium
        let calc = calculator();
        assert_eq!(calc.classify_level(0.25, 0.0), DriftLevel::Medium);
    }

    #[test]
    fn classify_level_high_at_4x_to_8x() {
        // kl = 5× threshold (0.5) → High
        let calc = calculator();
        assert_eq!(calc.classify_level(0.5, 0.0), DriftLevel::High);
    }

    #[test]
    fn classify_level_critical_at_8x_plus() {
        // kl = 10× threshold (1.0) → Critical
        let calc = calculator();
        assert_eq!(calc.classify_level(1.0, 0.0), DriftLevel::Critical);
    }

    #[test]
    fn classify_level_cosine_drives_when_higher() {
        // cosine = 9× threshold (1.35), kl = 0 → cosine drives → Critical
        let calc = calculator();
        assert_eq!(calc.classify_level(0.0, 1.35), DriftLevel::Critical);
    }

    /// Run where cosine direction drifts (embeddings pointing differently)
    /// but norms are equal — verifies cosine path is exercised in `compute()`.
    #[test]
    fn cosine_driven_drift_produces_nonzero_cosine_in_report() {
        let probe_id = ProbeId::new();
        let run_id = RunId::new();

        // Run embeddings point in a different direction than the baseline centroid
        let run = ProbeRun {
            id: run_id.clone(),
            probe_id: probe_id.clone(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            embeddings: vec![vec![0.0_f32, 1.0, 0.0], vec![0.0, 1.0, 0.0]],
            completions: vec!["hello world".into(), "hello world".into()],
            drift_report: None,
            status: RunStatus::Success,
        };
        // Baseline centroid points in the X direction
        let baseline = BaselineSnapshot {
            id: BaselineId::new(),
            probe_id,
            captured_at: Utc::now(),
            embedding_centroid: vec![1.0_f32, 0.0, 0.0],
            embedding_variance: 0.0,
            output_tokens: vec![
                vec!["hello".into(), "world".into()],
                vec!["hello".into(), "world".into()],
            ],
            run_id,
        };
        let report = calculator().compute(&run, &baseline).unwrap();
        // [0,1,0] vs [1,0,0] are orthogonal → cosine_distance = 0.5
        assert!(
            report.cosine_distance > 0.4,
            "expected strong cosine drift, got {}",
            report.cosine_distance
        );
    }

    /// A run with no completions should fail at the entropy step, not panic.
    #[test]
    fn run_with_empty_completions_returns_error() {
        let probe_id = ProbeId::new();
        let run_id = RunId::new();
        let run = ProbeRun {
            id: run_id.clone(),
            probe_id: probe_id.clone(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            embeddings: vec![vec![1.0_f32, 0.0, 0.0]],
            completions: vec![], // no completions → entropy_delta will fail
            drift_report: None,
            status: RunStatus::Success,
        };
        let baseline = BaselineSnapshot {
            id: BaselineId::new(),
            probe_id,
            captured_at: Utc::now(),
            embedding_centroid: vec![1.0_f32, 0.0, 0.0],
            embedding_variance: 0.0,
            output_tokens: vec![vec!["hello".into()]],
            run_id,
        };
        assert!(calculator().compute(&run, &baseline).is_err());
    }
}
