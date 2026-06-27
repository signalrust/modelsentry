//! Drift assessment: turns baseline + run output embeddings into a calibrated,
//! per-prompt drift verdict.
//!
//! This is the decision layer on top of [`crate::drift::twosample`]. It picks
//! the statistically appropriate test for the data available and produces a
//! single calibrated p-value plus a severity derived from the operator's target
//! false-positive rate.
//!
//! ### Two modes
//! - **Per-prompt conformal + stratified permutation** (preferred; needs ≥2
//!   baseline samples per prompt). Each prompt's run output is scored against its
//!   baseline cloud with a **conformal** (rank-based) p-value (exact,
//!   distribution-free, finite-sample valid under exchangeability — Vovk et al.;
//!   Lei et al., *JASA* 2018), reported per prompt for **attribution**. The
//!   run-level **gate** is a stratified permutation test of the aggregate
//!   statistic `T = Σ max(zᵢ, 0)` over standardized per-prompt excursions: each
//!   prompt's test point is exchangeable with its own baseline cloud, so
//!   resampling the test position *within each prompt* builds an exact null. The
//!   gate's resolution is `1/(B+1)` (B = permutations), so broad (model-wide)
//!   drift — the common case — clears small target FPRs that a single
//!   conformal rank (floored at `1/(k+1)`) could not.
//! - **Pooled two-sample** (fallback; single-sample baselines). Pools all
//!   prompts' outputs and runs one MMD/energy permutation test
//!   ([`crate::drift::twosample`]). Lower power, but valid with a single
//!   baseline run.
//!
//! In both modes the alert decision is `p < target_fpr`. The number of
//! permutations is auto-raised so `1/(B+1) ≤ target_fpr`, i.e. the gate can
//! always resolve the chosen FPR (it is never silently unable to alert).
//!
//! ### What the per-prompt floor does and does not limit
//! With a single run sample per prompt, *single-prompt-only* drift is bounded at
//! `1/(k+1)` — an information limit of one observation, sharpened only by a
//! larger baseline cloud `k`. Drift that moves several prompts together is not so
//! bounded: the aggregate gate combines their evidence and resolves well below
//! `1/(k+1)`.

// Sample counts cast to float for averaging/probabilities are always small
// (cloud sizes, prompt counts), so usize→f32/f64 precision loss is irrelevant.
#![allow(clippy::cast_precision_loss)]

use modelsentry_common::{
    error::{ModelSentryError, Result},
    models::DriftLevel,
};

use super::twosample::{self, Kernel, SplitMix64};

/// Variance floor for per-prompt standardization, so a near-deterministic
/// baseline cloud (temperature-0 / cached outputs) does not divide by ~0.
const STD_FLOOR: f32 = 1e-6;

/// Tolerance when counting permutation statistics `≥` the observed value.
const PERMUTATION_TOLERANCE: f32 = 1e-6;

/// A prompt with both a baseline cloud and ≥1 run sample, kept for assessment:
/// `(prompt_index, baseline_cloud, run_samples)`.
type UsablePrompt<'a> = (usize, &'a Vec<Vec<f32>>, &'a Vec<Vec<f32>>);

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
    /// Calibrated p-value for the whole run (stratified-permutation gate over the
    /// per-prompt scores, or the pooled permutation p-value in fallback mode).
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

/// Assess drift of `run` (a set of output embeddings per prompt) against
/// `baseline` (a cloud of output embeddings per prompt).
///
/// `baseline[i]` is prompt `i`'s baseline cloud; `run[i]` is prompt `i`'s set of
/// this-run output embeddings (empty ⇒ that prompt failed and is skipped). The
/// two outer slices must be the same length (same prompt set).
///
/// # Errors
///
/// - [`ModelSentryError::Provider`] if the prompt counts differ or no prompt has
///   usable data.
/// - [`ModelSentryError::DimensionMismatch`] if embedding dimensions differ.
pub fn assess(
    baseline: &[Vec<Vec<f32>>],
    run: &[Vec<Vec<f32>>],
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

    // Keep only prompts that have both a baseline cloud and ≥1 run sample.
    let usable: Vec<UsablePrompt<'_>> = baseline
        .iter()
        .zip(run.iter())
        .enumerate()
        .filter(|(_, (cloud, samples))| !cloud.is_empty() && !samples.is_empty())
        .map(|(i, (cloud, samples))| (i, cloud, samples))
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

/// Per-prompt assessment (clouds with ≥2 samples each).
///
/// Each prompt produces an attribution p-value, a standardized observed
/// excursion `z_obs`, and a null pool of standardized scores. The run-level
/// **gate** is a stratified permutation test of `T = Σ max(zᵢ, 0)` over the
/// per-prompt null pools (strata are independent under H0, so picking one score
/// per prompt builds an exact joint null). The gate's resolution is `1/(B+1)`.
///
/// Per prompt, the score depends on how many run samples are available:
/// - **≥2 run samples:** a proper two-sample energy permutation (cloud vs run
///   samples). When the run cluster is separated the observed energy exceeds
///   *every* permuted energy, so `z_obs` lies outside the null pool and the gate
///   reaches `1/(B+1)` — no `1/(k+1)` floor, even for a single drifted prompt.
/// - **1 run sample:** the rank-based conformal score (its observed value is
///   always reproducible by some permutation, so single-prompt drift is bounded
///   at `1/(k+1)` — an information limit of one observation).
fn assess_per_prompt(
    usable: &[UsablePrompt<'_>],
    config: &AssessmentConfig,
) -> Result<DriftAssessment> {
    // Resolution guard: the gate's smallest attainable p-value is 1/(B+1), so
    // ensure B can resolve the target FPR (the detector is never silently mute).
    let n_perm = config
        .n_permutations
        .max(min_perms_for_resolution(config.target_fpr));

    let mut per_prompt = Vec::with_capacity(usable.len());
    let mut strata: Vec<Vec<f32>> = Vec::with_capacity(usable.len());
    let mut observed = 0.0_f32;
    for (idx, cloud, samples) in usable {
        check_dims(cloud, samples)?;
        // Vary the seed per prompt so the strata are not permuted in lockstep.
        let seed = config.seed.wrapping_add(*idx as u64);
        let score = prompt_score(cloud, samples, n_perm, seed)?;
        per_prompt.push(PromptDrift {
            prompt_index: *idx,
            p_value: score.attribution_p,
            n_baseline: cloud.len(),
        });
        observed += score.z_obs.max(0.0);
        strata.push(score.null_pool);
    }

    let combined_p_value = stratified_permutation_p(&strata, observed, n_perm, config.seed);

    Ok(DriftAssessment {
        combined_p_value,
        statistic: drift_score(combined_p_value),
        level: severity_from_p(combined_p_value, config.target_fpr),
        method: METHOD_PER_PROMPT,
        per_prompt,
    })
}

/// One prompt's contribution to the gate: the standardized observed excursion,
/// the standardized null pool it is compared against, and the attribution
/// p-value reported per prompt.
struct PromptScore {
    z_obs: f32,
    null_pool: Vec<f32>,
    attribution_p: f32,
}

/// Score a prompt against its baseline cloud. Uses a two-sample energy
/// permutation when ≥2 run samples are available (no `1/(k+1)` floor), else the
/// rank-based conformal score for a single run sample.
fn prompt_score(
    cloud: &[Vec<f32>],
    samples: &[Vec<f32>],
    n_perm: usize,
    seed: u64,
) -> Result<PromptScore> {
    if samples.len() >= 2 {
        // Two-sample energy distance, standardized against its permutation null.
        let (observed, nulls) =
            twosample::permutation_nulls(cloud, samples, Kernel::Energy, n_perm, seed)?;
        let (mean, sd) = mean_std(&nulls);
        let at_least = nulls
            .iter()
            .filter(|&&e| e >= observed - PERMUTATION_TOLERANCE)
            .count();
        #[allow(clippy::cast_precision_loss)]
        let attribution_p = (1.0 + at_least as f32) / (1.0 + nulls.len() as f32);
        Ok(PromptScore {
            z_obs: (observed - mean) / sd,
            null_pool: nulls.iter().map(|e| (e - mean) / sd).collect(),
            attribution_p,
        })
    } else {
        // Single run sample → rank-based conformal score over the augmented set.
        let scores = augmented_scores(&samples[0], cloud);
        let z = standardize(&scores);
        Ok(PromptScore {
            z_obs: z.first().copied().unwrap_or(0.0),
            null_pool: z,
            attribution_p: conformal_from_scores(&scores),
        })
    }
}

/// Mean and standard deviation (variance floored by [`STD_FLOOR`]) of a slice.
fn mean_std(xs: &[f32]) -> (f32, f32) {
    let n = xs.len() as f32;
    if n == 0.0 {
        return (0.0, STD_FLOOR);
    }
    let mean = xs.iter().sum::<f32>() / n;
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / n;
    (mean, var.sqrt().max(STD_FLOOR))
}

/// Stratified permutation p-value of the aggregate statistic `T = Σ max(zᵢ, 0)`.
///
/// `strata[i]` holds prompt `i`'s standardized augmented scores (index 0 is the
/// observed run/test point). Under H0 the test point is exchangeable with the
/// baseline points within each prompt, so each permutation draws one position
/// per prompt — *including* the observed one, which keeps the estimate valid and
/// floors the p-value at `1/(n_perm + 1)`.
fn stratified_permutation_p(strata: &[Vec<f32>], observed: f32, n_perm: usize, seed: u64) -> f32 {
    let mut rng = SplitMix64::new(seed);
    let mut at_least = 0usize;
    for _ in 0..n_perm {
        let mut t = 0.0_f32;
        for pool in strata {
            if pool.is_empty() {
                continue;
            }
            let pick = pool
                .get(rng.next_bounded(pool.len()))
                .copied()
                .unwrap_or(0.0);
            t += pick.max(0.0);
        }
        if t >= observed - PERMUTATION_TOLERANCE {
            at_least += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let p = (1.0 + at_least as f32) / (1.0 + n_perm as f32);
    p
}

/// Minimum permutations so the gate's grid (`1/(B+1)`) can resolve `target_fpr`.
fn min_perms_for_resolution(target_fpr: f32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let needed = (1.0 / target_fpr.max(f32::MIN_POSITIVE)).ceil() as usize;
    needed
}

/// Augmented-set scores for one prompt: each of the `k + 1` points in
/// `cloud ∪ {point}` scores its mean distance to the *other* `k` points (so every
/// score uses the same reference-set size — the condition for exact conformal
/// validity). Index 0 is the run (test) point; indices `1..=k` are the cloud.
fn augmented_scores(point: &[f32], cloud: &[Vec<f32>]) -> Vec<f32> {
    let k = cloud.len();
    let mut scores = Vec::with_capacity(k + 1);
    // Test point: mean distance to all k cloud points.
    scores.push(cloud.iter().map(|c| euclidean(point, c)).sum::<f32>() / k as f32);
    // Each cloud point: mean distance to its k others (other cloud points + test).
    for (j, cj) in cloud.iter().enumerate() {
        let to_other_cloud: f32 = cloud
            .iter()
            .enumerate()
            .filter(|(l, _)| *l != j)
            .map(|(_, cl)| euclidean(cj, cl))
            .sum();
        scores.push((to_other_cloud + euclidean(cj, point)) / k as f32);
    }
    scores
}

/// Conformal (rank-based) p-value from augmented `scores` (index 0 = test):
/// `p = (1 + #{cloud score ≥ test score}) / (k + 1)`, super-uniform under
/// exchangeability. Reported per prompt for attribution.
fn conformal_from_scores(scores: &[f32]) -> f32 {
    let test = scores.first().copied().unwrap_or(0.0);
    let k = scores.len().saturating_sub(1);
    let at_least = scores.iter().skip(1).filter(|&&s| s >= test).count();
    #[allow(clippy::cast_precision_loss)]
    let p = (1.0 + at_least as f32) / (1.0 + k as f32);
    p
}

/// Standardize scores to zero mean / unit variance over the augmented set, so
/// prompts with different output-distance scales are comparable. The variance is
/// floored ([`STD_FLOOR`]) for near-deterministic baselines.
fn standardize(scores: &[f32]) -> Vec<f32> {
    let n = scores.len() as f32;
    if n == 0.0 {
        return Vec::new();
    }
    let mean = scores.iter().sum::<f32>() / n;
    let var = scores.iter().map(|s| (s - mean) * (s - mean)).sum::<f32>() / n;
    let sd = var.sqrt().max(STD_FLOOR);
    scores.iter().map(|s| (s - mean) / sd).collect()
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
        .flat_map(|(_, _, samples)| samples.iter().cloned())
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

/// Validate that a cloud and all run samples share one dimension.
fn check_dims(cloud: &[Vec<f32>], samples: &[Vec<f32>]) -> Result<()> {
    let dim = samples.first().map_or(0, Vec::len);
    for v in cloud.iter().chain(samples.iter()) {
        if v.len() != dim {
            return Err(ModelSentryError::DimensionMismatch {
                expected: dim,
                actual: v.len(),
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

    /// Wrap a single embedding as a one-sample run for a prompt (n = 1 path).
    fn one(p: Vec<f32>) -> Vec<Vec<f32>> {
        vec![p]
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
        let run = vec![one(point(dim, 0.0)), one(point(dim, 25.0))];
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
        let run = vec![one(point(dim, 0.02)), one(point(dim, 5.01))];
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
        let run = vec![one(point(dim, 0.0)), one(point(dim, 5.0))];
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
        let run = vec![one(point(2, 0.0)), one(point(2, 1.0))];
        assert!(assess(&baseline, &run, &AssessmentConfig::default()).is_err());
    }

    #[test]
    fn min_perms_for_resolution_supports_the_target_fpr() {
        // Need 1/(B+1) <= alpha, i.e. B >= ceil(1/alpha).
        assert!(min_perms_for_resolution(0.01) >= 100);
        assert!(min_perms_for_resolution(0.05) >= 20);
    }

    // ── Calibration & power ──────────────────────────────────────────────────

    /// Tiny deterministic xorshift PRNG for generating test data (no `rand`).
    struct XorShift(u64);
    impl XorShift {
        fn unit(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            // Top 24 bits → [0, 1).
            ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
        }
        /// Approx standard normal via the Irwin–Hall sum (12 uniforms − 6).
        fn normal(&mut self) -> f32 {
            (0..12).map(|_| self.unit()).sum::<f32>() - 6.0
        }
        fn normal_vec(&mut self, d: usize) -> Vec<f32> {
            (0..d).map(|_| self.normal()).collect()
        }
    }

    /// Regression guard for the un-fireable-default bug: under the null the gate
    /// must (a) actually be able to fire and (b) stay calibrated near the target
    /// FPR — not "always None" (the old Šidák-of-min behaviour at k=20, α=0.01).
    #[test]
    fn gate_is_calibrated_under_the_null() {
        let (d, k, prompts, trials) = (8usize, 20usize, 3usize, 300usize);
        let alpha = 0.1_f32;
        let mut rng = XorShift(0xC0FF_EE12_3456_789A);
        let mut fired = 0usize;
        for t in 0..trials {
            let baseline: Vec<Vec<Vec<f32>>> = (0..prompts)
                .map(|_| (0..k).map(|_| rng.normal_vec(d)).collect())
                .collect();
            // 3 samples per prompt from the SAME distribution ⇒ no drift (null);
            // exercises the two-sample (n ≥ 2) path under the null.
            let run: Vec<Vec<Vec<f32>>> = (0..prompts)
                .map(|_| (0..3).map(|_| rng.normal_vec(d)).collect())
                .collect();
            let cfg = AssessmentConfig {
                target_fpr: alpha,
                n_permutations: 200,
                seed: t as u64,
                ..AssessmentConfig::default()
            };
            let out = assess(&baseline, &run, &cfg).unwrap();
            if out.combined_p_value < alpha {
                fired += 1;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let empirical = fired as f32 / trials as f32;
        // Calibrated: empirical FPR ≈ α (valid ⇒ not much above; not the old
        // pathological 0). Generous band for 300 Monte-Carlo trials.
        assert!(
            empirical > 0.0,
            "detector must be able to fire (un-fireable-default regression)"
        );
        assert!(
            empirical <= 1.7 * alpha,
            "empirical FPR {empirical} should be ~≤ target {alpha} (calibrated)"
        );
    }

    /// Broad (model-wide) drift must clear `target_fpr = 0.01` even though the
    /// single-prompt conformal rank is floored at `1/(k+1) ≈ 0.048` for k = 20 —
    /// i.e. the aggregate gate resolves below the per-prompt floor.
    #[test]
    fn broad_drift_clears_target_below_single_prompt_floor() {
        let (d, k) = (6usize, 20usize);
        let baseline = vec![
            cloud(k, d, 0.0, 0.1),
            cloud(k, d, 5.0, 0.1),
            cloud(k, d, -3.0, 0.1),
        ];
        // Every prompt's output lands well outside its baseline cloud.
        let run = vec![one(point(d, 4.0)), one(point(d, 9.0)), one(point(d, 2.0))];
        let cfg = AssessmentConfig {
            target_fpr: 0.01,
            n_permutations: 500,
            seed: 1,
            ..AssessmentConfig::default()
        };
        let out = assess(&baseline, &run, &cfg).unwrap();
        assert_eq!(out.method, METHOD_PER_PROMPT);
        assert!(
            out.combined_p_value < 0.01,
            "broad drift should clear 0.01 (single-prompt floor is ~0.048), got {}",
            out.combined_p_value
        );
        // Drift is detected; severity magnitude is bounded by the gate's
        // resolution (1/(B+1)), so it need not reach Critical here.
        assert_ne!(out.level, DriftLevel::None);
    }

    /// The multi-sample payoff: with ≥2 run samples per prompt, even a *single*
    /// drifted prompt clears `target_fpr = 0.01` — below the `1/(k+1) ≈ 0.048`
    /// conformal floor that bounds the single-sample case (k = 20).
    #[test]
    fn multi_sample_single_prompt_drift_beats_the_conformal_floor() {
        let (d, k) = (5usize, 20usize);
        let baseline = vec![cloud(k, d, 0.0, 0.1), cloud(k, d, 5.0, 0.1)];
        // Prompt 0 stable (samples inside its cloud); prompt 1 drifts (a tight
        // cluster far outside its cloud) — only ONE prompt moved.
        let run = vec![
            vec![point(d, 0.02), point(d, -0.01), point(d, 0.0)],
            vec![point(d, 20.0), point(d, 20.1), point(d, 19.9)],
        ];
        let cfg = AssessmentConfig {
            target_fpr: 0.01,
            n_permutations: 300,
            seed: 3,
            ..AssessmentConfig::default()
        };
        let out = assess(&baseline, &run, &cfg).unwrap();
        assert_eq!(out.method, METHOD_PER_PROMPT);
        assert!(
            out.combined_p_value < 0.01,
            "single-prompt drift with multi-sampling should clear 0.01 (conformal \
             floor ~0.048), got {}",
            out.combined_p_value
        );
        assert_ne!(out.level, DriftLevel::None);
    }
}
