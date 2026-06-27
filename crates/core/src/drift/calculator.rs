//! Drift calculator: runs the calibrated two-sample assessment for a probe run
//! against its baseline and packages the result as a [`DriftReport`].

use chrono::Utc;
use modelsentry_common::{
    error::{ModelSentryError, Result},
    models::{BaselineSnapshot, DriftReport, ProbeRun, PromptDrift},
};

use super::assessment::{self, AssessmentConfig};
use super::interpret;

/// Produces a calibrated [`DriftReport`] from a probe run and its baseline,
/// using the per-prompt conformal / pooled two-sample assessment.
#[derive(Debug, Clone)]
pub struct DriftCalculator {
    config: AssessmentConfig,
}

impl DriftCalculator {
    /// Create a calculator with the given assessment configuration (target FPR,
    /// kernel, permutations, seed).
    #[must_use]
    pub fn new(config: AssessmentConfig) -> Self {
        Self { config }
    }

    /// The target false-positive rate this calculator assesses against.
    #[must_use]
    pub fn target_fpr(&self) -> f32 {
        self.config.target_fpr
    }

    /// Compute a calibrated [`DriftReport`] comparing `run` (one output
    /// embedding per prompt) to `baseline` (per-prompt output-embedding clouds).
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Config`] if the baseline uses an outdated schema and
    ///   must be re-captured.
    /// - [`ModelSentryError::EmptyEmbedding`] if the run produced no embeddings.
    /// - [`ModelSentryError::BaselineEmbeddingMismatch`] if the embedding model
    ///   changed since the baseline was captured.
    /// - Propagates assessment errors (e.g. no usable prompts).
    pub fn compute(&self, run: &ProbeRun, baseline: &BaselineSnapshot) -> Result<DriftReport> {
        if !baseline.is_current() {
            return Err(ModelSentryError::Config {
                message: "baseline uses an outdated schema (captured before output-embedding \
                          clouds) — re-capture the baseline for this probe"
                    .to_string(),
            });
        }

        // Embedding-model migration guard (see A4): dimensions must match.
        let run_dim = run
            .embeddings
            .iter()
            .flat_map(|samples| samples.iter())
            .find(|e| !e.is_empty())
            .map_or(0, Vec::len);
        if run_dim == 0 {
            return Err(ModelSentryError::EmptyEmbedding);
        }
        let baseline_dim = baseline.embedding_dim();
        if baseline_dim != 0 && run_dim != baseline_dim {
            return Err(ModelSentryError::BaselineEmbeddingMismatch {
                baseline_dim,
                run_dim,
            });
        }

        let assessment =
            assessment::assess(&baseline.prompt_clouds, &run.embeddings, &self.config)?;
        let interpretation = interpret::interpret(&assessment, self.config.target_fpr);
        let per_prompt = assessment
            .per_prompt
            .iter()
            .map(|p| PromptDrift {
                prompt_index: p.prompt_index,
                p_value: p.p_value,
                n_baseline: p.n_baseline,
            })
            .collect();

        Ok(DriftReport {
            run_id: run.id.clone(),
            baseline_id: baseline.id.clone(),
            combined_p_value: assessment.combined_p_value,
            statistic: assessment.statistic,
            target_fpr: self.config.target_fpr,
            method: assessment.method.to_string(),
            per_prompt,
            drift_level: assessment.level,
            interpretation,
            computed_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelsentry_common::{
        models::{BASELINE_SCHEMA_VERSION, BaselineSnapshot, DriftLevel, ProbeRun, RunStatus},
        types::{BaselineId, ProbeId, RunId},
    };

    fn run_with(embeddings: Vec<Vec<f32>>) -> ProbeRun {
        ProbeRun {
            id: RunId::new(),
            probe_id: ProbeId::new(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            // One sample per prompt (the n=1 path).
            embeddings: embeddings.into_iter().map(|e| vec![e]).collect(),
            completions: vec!["answer".to_string()],
            drift_report: None,
            status: RunStatus::Success,
        }
    }

    fn baseline_with(clouds: Vec<Vec<Vec<f32>>>) -> BaselineSnapshot {
        BaselineSnapshot {
            id: BaselineId::new(),
            probe_id: ProbeId::new(),
            captured_at: Utc::now(),
            schema_version: BASELINE_SCHEMA_VERSION,
            embedding_model: "test".to_string(),
            prompt_clouds: clouds,
            n_runs: 1,
            run_id: RunId::new(),
        }
    }

    fn axis_point(dim: usize, x0: f32) -> Vec<f32> {
        let mut v = vec![0.0; dim];
        if let Some(f) = v.first_mut() {
            *f = x0;
        }
        v
    }

    fn cloud(count: usize, dim: usize, center: f32) -> Vec<Vec<f32>> {
        (0..count)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let off = (i as f32 - count as f32 / 2.0) * 0.02;
                axis_point(dim, center + off)
            })
            .collect()
    }

    #[test]
    fn stable_run_reports_no_drift() {
        let calc = DriftCalculator::new(AssessmentConfig::default());
        let baseline = baseline_with(vec![cloud(15, 4, 0.0)]);
        let run = run_with(vec![axis_point(4, 0.01)]);
        let report = calc.compute(&run, &baseline).unwrap();
        assert_eq!(report.drift_level, DriftLevel::None);
        assert!(report.combined_p_value > 0.05);
        assert!(!report.interpretation.is_empty());
    }

    #[test]
    fn drifted_run_reports_drift() {
        let calc = DriftCalculator::new(AssessmentConfig {
            target_fpr: 0.05,
            ..AssessmentConfig::default()
        });
        let baseline = baseline_with(vec![cloud(60, 4, 0.0), cloud(60, 4, 10.0)]);
        let run = run_with(vec![axis_point(4, 0.0), axis_point(4, 25.0)]);
        let report = calc.compute(&run, &baseline).unwrap();
        assert!(
            report.combined_p_value < 0.05,
            "p={}",
            report.combined_p_value
        );
        assert_ne!(report.drift_level, DriftLevel::None);
    }

    #[test]
    fn outdated_baseline_schema_is_rejected() {
        let calc = DriftCalculator::new(AssessmentConfig::default());
        let mut baseline = baseline_with(vec![cloud(5, 4, 0.0)]);
        baseline.schema_version = 1; // legacy
        let run = run_with(vec![axis_point(4, 0.0)]);
        let err = calc.compute(&run, &baseline).unwrap_err();
        assert!(err.to_string().contains("re-capture"), "{err}");
    }

    #[test]
    fn dimension_mismatch_is_flagged() {
        let calc = DriftCalculator::new(AssessmentConfig::default());
        let baseline = baseline_with(vec![cloud(5, 3, 0.0)]);
        let run = run_with(vec![axis_point(4, 0.0)]); // dim 4 vs baseline dim 3
        let err = calc.compute(&run, &baseline).unwrap_err();
        assert!(matches!(
            err,
            ModelSentryError::BaselineEmbeddingMismatch {
                baseline_dim: 3,
                run_dim: 4,
            }
        ));
    }
}
