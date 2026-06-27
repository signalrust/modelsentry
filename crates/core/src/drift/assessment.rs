//! Drift assessment: turns baseline + run output embeddings into a calibrated,
//! per-prompt drift verdict.
//!
//! This is the decision layer on top of [`crate::drift::twosample`]. It picks
//! the statistically appropriate test for the data available and produces a
//! single calibrated p-value plus a severity derived from the operator's target
//! false-positive rate.
//!
//! ### Two modes
//! - **Per-prompt conformal** (preferred; needs ≥2 baseline samples per prompt).
//!   For each prompt, the run's new output embedding is scored against that
//!   prompt's baseline cloud using a **conformal** (rank-based) p-value — exact,
//!   distribution-free, finite-sample valid under exchangeability (Vovk et al.;
//!   Lei et al., *JASA* 2018). Per-prompt p-values are combined with the
//!   **Šidák** correction on the minimum, which controls the family-wise error
//!   rate and stays powerful when a *single* prompt drifts (the common failure
//!   mode that Fisher's method would dilute).
//! - **Pooled two-sample** (fallback; single-sample baselines). Pools all
//!   prompts' outputs and runs one MMD/energy permutation test
//!   ([`crate::drift::twosample`]). Lower power, but valid with a single
//!   baseline run.
//!
//! In both modes the alert decision is `p < target_fpr`, so the threshold *is*
//! the calibrated false-positive rate.

// Sample counts cast to float for averaging/probabilities are always small
// (cloud sizes, prompt counts), so usize→f32/f64 precision loss is irrelevant.
#![allow(clippy::cast_precision_loss)]

use modelsentry_common::{
    error::{ModelSentryError, Result},
    models::DriftLevel,
};

use super::twosample::{self, Kernel};

/// A prompt with both a baseline cloud and a run output, kept for assessment:
/// `(prompt_index, baseline_cloud, run_embedding)`.
type UsablePrompt<'a> = (usize, &'a Vec<Vec<f32>>, &'a Vec<f32>);

/// Tuning for a drift assessment.
#[derive(Debug, Clone, Copy)]
pub struct AssessmentConfig {
    /// Target false-positive rate. The probe alerts when the combined p-value is
    /// below this. Severity bands are defined relative to it (orders of
    /// magnitude below ⇒ more severe).
    pub target_fpr: f32,
    /// Kernel for the pooled-fallback two-sample test.
    pub kernel: Kernel,
    /// Permutations for the pooled-fallback test.
    pub n_permutations: usize,
    /// Seed for deterministic, reproducible results.
    pub seed: u64,
}

impl Default for AssessmentConfig {
    fn default() -> Self {
        Self {
            target_fpr: 0.01,
            kernel: Kernel::rbf_median(),
            n_permutations: 200,
            seed: 0,
        }
    }
}

/// Per-prompt conformal result.
#[derive(Debug, Clone, Copy)]
pub struct PromptDrift {
    /// Index of the prompt within the probe.
    pub prompt_index: usize,
    /// Conformal p-value for this prompt (lower ⇒ more anomalous).
    pub p_value: f32,
    /// Baseline cloud size used for this prompt.
    pub n_baseline: usize,
}

/// Full drift verdict for a run.
#[derive(Debug, Clone)]
pub struct DriftAssessment {
    /// Calibrated p-value for the whole run (Šidák-combined per-prompt, or the
    /// pooled permutation p-value in fallback mode).
    pub combined_p_value: f32,
    /// Drift score = `−log₁₀(combined_p_value)`; higher ⇒ stronger evidence.
    /// Monotone in significance and consistent across both modes.
    pub statistic: f32,
    /// Severity derived from `combined_p_value` vs `target_fpr`.
    pub level: DriftLevel,
    /// Which test produced the verdict.
    pub method: &'static str,
    /// Per-prompt breakdown (empty in pooled mode).
    pub per_prompt: Vec<PromptDrift>,
}

/// Method tag: per-prompt conformal test. Sourced from the workspace SSOT.
pub const METHOD_PER_PROMPT: &str = modelsentry_common::constants::method::PER_PROMPT_CONFORMAL;
/// Method tag: pooled MMD/energy two-sample test. Sourced from the workspace SSOT.
pub const METHOD_POOLED: &str = modelsentry_common::constants::method::POOLED_TWO_SAMPLE;

/// Assess drift of `run` (one output embedding per prompt) against `baseline`
/// (a cloud of output embeddings per prompt).
///
/// `baseline[i]` is prompt `i`'s baseline cloud; `run[i]` is prompt `i`'s output
/// embedding this run (empty ⇒ that prompt failed and is skipped). The two
/// outer slices must be the same length (same prompt set).
///
/// # Errors
///
/// - [`ModelSentryError::Provider`] if the prompt counts differ or no prompt has
///   usable data.
/// - [`ModelSentryError::DimensionMismatch`] if embedding dimensions differ.
pub fn assess(
    baseline: &[Vec<Vec<f32>>],
    run: &[Vec<f32>],
    config: &AssessmentConfig,
) -> Result<DriftAssessment> {
    if baseline.len() != run.len() {
        return Err(ModelSentryError::Provider {
            message: format!(
                "baseline/run prompt count mismatch: {} vs {}",
                baseline.len(),
                run.len()
            ),
        });
    }

    // Keep only prompts that have both a baseline cloud and a run output.
    let usable: Vec<UsablePrompt<'_>> = baseline
        .iter()
        .zip(run.iter())
        .enumerate()
        .filter(|(_, (cloud, point))| !cloud.is_empty() && !point.is_empty())
        .map(|(i, (cloud, point))| (i, cloud, point))
        .collect();

    if usable.is_empty() {
        return Err(ModelSentryError::Provider {
            message: "no prompt has both baseline and run embeddings".to_string(),
        });
    }

    let min_cloud = usable.iter().map(|(_, c, _)| c.len()).min().unwrap_or(0);

    if min_cloud >= 2 {
        assess_per_prompt(&usable, config)
    } else {
        assess_pooled(&usable, config)
    }
}

/// Per-prompt conformal assessment (clouds with ≥2 samples each).
fn assess_per_prompt(
    usable: &[UsablePrompt<'_>],
    config: &AssessmentConfig,
) -> Result<DriftAssessment> {
    let mut per_prompt = Vec::with_capacity(usable.len());
    let mut p_values = Vec::with_capacity(usable.len());
    for (idx, cloud, point) in usable {
        check_dims(cloud, point)?;
        let p = conformal_p_value(point, cloud);
        per_prompt.push(PromptDrift {
            prompt_index: *idx,
            p_value: p,
            n_baseline: cloud.len(),
        });
        p_values.push(p);
    }

    let combined_p_value = sidak_combined_p(&p_values);
    Ok(DriftAssessment {
        combined_p_value,
        statistic: drift_score(combined_p_value),
        level: severity_from_p(combined_p_value, config.target_fpr),
        method: METHOD_PER_PROMPT,
        per_prompt,
    })
}

/// Pooled MMD/energy fallback (single-sample baselines).
fn assess_pooled(
    usable: &[UsablePrompt<'_>],
    config: &AssessmentConfig,
) -> Result<DriftAssessment> {
    let baseline_points: Vec<Vec<f32>> = usable
        .iter()
        .flat_map(|(_, cloud, _)| cloud.iter().cloned())
        .collect();
    let run_points: Vec<Vec<f32>> = usable
        .iter()
        .map(|(_, _, point)| (*point).clone())
        .collect();

    let outcome = twosample::two_sample_test(
        &baseline_points,
        &run_points,
        config.kernel,
        config.n_permutations,
        config.seed,
    )?;

    Ok(DriftAssessment {
        combined_p_value: outcome.p_value,
        statistic: drift_score(outcome.p_value),
        level: severity_from_p(outcome.p_value, config.target_fpr),
        method: METHOD_POOLED,
        per_prompt: Vec::new(),
    })
}

/// Conformal (rank-based) p-value of a single `point` against a reference
/// `cloud`, using mean-distance nonconformity over the **augmented** set
/// `cloud ∪ {point}`.
///
/// Each of the `k + 1` augmented points scores its mean distance to the *other*
/// `k` points (so every score uses the same reference-set size — the condition
/// for exact conformal validity). The p-value is the rank of the test point:
/// `p = (1 + #{cloud score ≥ test score}) / (k + 1)`, which is super-uniform
/// under exchangeability. Requires `cloud.len() ≥ 2`.
fn conformal_p_value(point: &[f32], cloud: &[Vec<f32>]) -> f32 {
    let k = cloud.len();
    // Test point: mean distance to all k cloud points (its k "others" in the
    // augmented set).
    let test_score = cloud.iter().map(|c| euclidean(point, c)).sum::<f32>() / k as f32;

    // Calibration: each cloud point's mean distance to its k others in the
    // augmented set = the other (k − 1) cloud points **plus the test point**.
    let mut at_least = 0usize;
    for (j, cj) in cloud.iter().enumerate() {
        let sum_to_other_cloud: f32 = cloud
            .iter()
            .enumerate()
            .filter(|(l, _)| *l != j)
            .map(|(_, cl)| euclidean(cj, cl))
            .sum();
        let cal_score = (sum_to_other_cloud + euclidean(cj, point)) / k as f32;
        if cal_score >= test_score {
            at_least += 1;
        }
    }
    (1.0 + at_least as f32) / (1.0 + k as f32)
}

/// Combine per-prompt p-values via the **Šidák** correction on the minimum:
/// `1 − (1 − p_min)^P`. Controls the family-wise error rate, so a single drifted
/// prompt alerts at the target FPR (unlike Fisher, which dilutes it).
fn sidak_combined_p(p_values: &[f32]) -> f32 {
    if p_values.is_empty() {
        return 1.0;
    }
    let p_min = p_values.iter().copied().fold(1.0_f32, f32::min);
    let exp = i32::try_from(p_values.len()).unwrap_or(i32::MAX);
    let combined = 1.0 - (1.0 - f64::from(p_min)).powi(exp);
    #[allow(clippy::cast_possible_truncation)]
    let out = combined.clamp(0.0, 1.0) as f32;
    out
}

/// `−log₁₀(p)`, floored so a zero p-value does not produce infinity.
fn drift_score(p: f32) -> f32 {
    -(p.max(f32::MIN_POSITIVE)).log10()
}

/// Map a calibrated p-value to a [`DriftLevel`] using `target_fpr` as the
/// significance threshold; each additional order of magnitude below it escalates
/// the severity.
fn severity_from_p(p: f32, target_fpr: f32) -> DriftLevel {
    let alpha = target_fpr.max(f32::MIN_POSITIVE);
    if p >= alpha {
        DriftLevel::None
    } else if p >= alpha / 10.0 {
        DriftLevel::Low
    } else if p >= alpha / 100.0 {
        DriftLevel::Medium
    } else if p >= alpha / 1000.0 {
        DriftLevel::High
    } else {
        DriftLevel::Critical
    }
}

/// Euclidean distance between two equal-length vectors.
fn euclidean(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
}

/// Validate a cloud and point share one dimension.
fn check_dims(cloud: &[Vec<f32>], point: &[f32]) -> Result<()> {
    let dim = point.len();
    for c in cloud {
        if c.len() != dim {
            return Err(ModelSentryError::DimensionMismatch {
                expected: dim,
                actual: c.len(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One baseline cloud per prompt: `count` points around `center` on axis 0.
    fn cloud(count: usize, dim: usize, center: f32, jitter: f32) -> Vec<Vec<f32>> {
        (0..count)
            .map(|i| {
                let off = (i as f32 - count as f32 / 2.0) * jitter;
                let mut v = vec![0.0; dim];
                if let Some(first) = v.first_mut() {
                    *first = center + off;
                }
                v
            })
            .collect()
    }

    fn point(dim: usize, x0: f32) -> Vec<f32> {
        let mut v = vec![0.0; dim];
        if let Some(f) = v.first_mut() {
            *f = x0;
        }
        v
    }

    #[test]
    fn sidak_single_prompt_returns_that_p_value() {
        // For one prompt, Šidák reduces to the input p.
        assert!((sidak_combined_p(&[0.2]) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn sidak_is_sensitive_to_a_single_small_p() {
        // One strong prompt among quiet ones still drives the combined p down,
        // unlike Fisher which would average it away.
        let combined = sidak_combined_p(&[0.01, 0.6, 0.7, 0.5]);
        assert!(
            combined < 0.05,
            "Šidák should stay sensitive, got {combined}"
        );
    }

    #[test]
    fn severity_bands_track_orders_of_magnitude() {
        let a = 0.01;
        assert_eq!(severity_from_p(0.5, a), DriftLevel::None);
        assert_eq!(severity_from_p(0.005, a), DriftLevel::Low);
        assert_eq!(severity_from_p(0.0005, a), DriftLevel::Medium);
        assert_eq!(severity_from_p(0.00005, a), DriftLevel::High);
        assert_eq!(severity_from_p(1e-6, a), DriftLevel::Critical);
    }

    #[test]
    fn per_prompt_mode_flags_a_clearly_drifted_prompt() {
        let dim = 4;
        // Two prompts with tight baseline clouds; large K so a single drifted
        // prompt can clear the target FPR.
        let baseline = vec![cloud(60, dim, 0.0, 0.05), cloud(60, dim, 10.0, 0.05)];
        // Prompt 0 stays put; prompt 1 jumps far from its cloud.
        let run = vec![point(dim, 0.0), point(dim, 25.0)];
        let cfg = AssessmentConfig {
            target_fpr: 0.05,
            ..AssessmentConfig::default()
        };
        let out = assess(&baseline, &run, &cfg).unwrap();
        assert_eq!(out.method, METHOD_PER_PROMPT);
        assert!(
            out.combined_p_value < 0.05,
            "expected drift, p={}",
            out.combined_p_value
        );
        assert_ne!(out.level, DriftLevel::None);
        assert_eq!(out.per_prompt.len(), 2);
    }

    #[test]
    fn per_prompt_mode_is_quiet_when_stable() {
        let dim = 4;
        let baseline = vec![cloud(15, dim, 0.0, 0.1), cloud(15, dim, 5.0, 0.1)];
        // Run outputs sit comfortably inside each baseline cloud.
        let run = vec![point(dim, 0.02), point(dim, 5.01)];
        let out = assess(&baseline, &run, &AssessmentConfig::default()).unwrap();
        assert!(
            out.combined_p_value > 0.05,
            "should be quiet, p={}",
            out.combined_p_value
        );
        assert_eq!(out.level, DriftLevel::None);
    }

    #[test]
    fn single_sample_baseline_falls_back_to_pooled() {
        let dim = 4;
        let baseline = vec![cloud(1, dim, 0.0, 0.0), cloud(1, dim, 5.0, 0.0)];
        let run = vec![point(dim, 0.0), point(dim, 5.0)];
        let out = assess(&baseline, &run, &AssessmentConfig::default()).unwrap();
        assert_eq!(out.method, METHOD_POOLED);
    }

    #[test]
    fn errors_when_no_usable_prompts() {
        let baseline = vec![vec![], vec![]];
        let run = vec![vec![], vec![]];
        assert!(assess(&baseline, &run, &AssessmentConfig::default()).is_err());
    }

    #[test]
    fn errors_on_prompt_count_mismatch() {
        let baseline = vec![cloud(3, 2, 0.0, 0.1)];
        let run = vec![point(2, 0.0), point(2, 1.0)];
        assert!(assess(&baseline, &run, &AssessmentConfig::default()).is_err());
    }
}
