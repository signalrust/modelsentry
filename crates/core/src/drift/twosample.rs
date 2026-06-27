//! Nonparametric two-sample tests for embedding drift.
//!
//! Given a **baseline** set of output embeddings `B = {b_1, …, b_m}` and a
//! **run** set `R = {r_1, …, r_n}`, we ask: *were these two samples drawn from
//! the same distribution?* This is the kernel two-sample testing problem.
//!
//! Two complementary statistics are provided, both members of the same family:
//!
//! - **MMD²** — squared Maximum Mean Discrepancy with an RBF kernel
//!   (Gretton et al., *A Kernel Two-Sample Test*, JMLR 2012). The unbiased
//!   estimator is used. Bandwidth defaults to the **median heuristic** over the
//!   pooled sample.
//! - **Energy distance** (Székely & Rizzo, 2013) — parameter-free; it is exactly
//!   MMD with the negative-distance kernel `k(a,b) = −‖a − b‖`, so it shares the
//!   same machinery with no bandwidth to tune.
//!
//! ### Why not covariance-aware Gaussian KL?
//! A closed-form multivariate Gaussian KL needs each set's `d × d` covariance.
//! Embeddings are high-dimensional (`d` = 1536 / 3072) while a probe yields only
//! a handful of samples per run (`n ≪ d`), so the covariance is singular and the
//! KL is undefined / unstable. Kernel methods are nonparametric and valid for
//! `n ≪ d`, which is why they are the right tool here.
//!
//! ### Calibration to a false-positive rate
//! The raw statistic is not directly interpretable, so significance is obtained
//! by a **permutation test**: pool the `m + n` points, repeatedly relabel them
//! into groups of size `m` and `n`, and compare the observed statistic to this
//! permutation null. Under the null hypothesis (no drift) the resulting p-value
//! is ~Uniform(0, 1), so alerting when `p < α` yields a false-positive rate of
//! approximately `α` — the threshold *is* the target FPR, by construction.

use modelsentry_common::error::{ModelSentryError, Result};

/// Minimum samples per group required for the unbiased MMD² estimator
/// (`1 / (m (m − 1))` needs `m ≥ 2`).
const MIN_SAMPLES_PER_GROUP: usize = 2;

/// Floor applied to the RBF bandwidth so identical pooled points (median
/// pairwise distance 0) do not produce a degenerate kernel.
const BANDWIDTH_FLOOR: f32 = 1e-6;

/// Tolerance when counting permutation statistics `≥` the observed value.
const PERMUTATION_TOLERANCE: f32 = 1e-6;

/// Kernel choice for the two-sample statistic.
#[derive(Debug, Clone, Copy)]
pub enum Kernel {
    /// Gaussian RBF kernel `exp(−‖a − b‖² / (2 σ²))`. A non-finite or
    /// non-positive bandwidth is replaced by the pooled median heuristic.
    Rbf { bandwidth: f32 },
    /// Negative-distance kernel `−‖a − b‖`, giving the energy distance.
    Energy,
}

impl Kernel {
    /// RBF kernel whose bandwidth is chosen at test time by the median
    /// heuristic over the pooled sample.
    #[must_use]
    pub fn rbf_median() -> Self {
        Self::Rbf {
            bandwidth: f32::NAN,
        }
    }
}

/// Outcome of a permutation two-sample test.
#[derive(Debug, Clone, Copy)]
pub struct TwoSampleOutcome {
    /// Observed statistic (unbiased MMD² for RBF, energy distance for Energy).
    /// Larger ⇒ more evidence the two samples differ.
    pub statistic: f32,
    /// Permutation p-value in `[0, 1]`. Calibrated so that thresholding at `α`
    /// gives a false-positive rate of ≈ `α`. Lower ⇒ stronger drift evidence.
    pub p_value: f32,
    /// Baseline sample size.
    pub n_baseline: usize,
    /// Run sample size.
    pub n_run: usize,
    /// Number of permutations used to build the null distribution.
    pub n_permutations: usize,
}

/// Squared Euclidean distance between two equal-length vectors.
fn sq_dist(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// Median of the pairwise Euclidean distances over `points` — the classic RBF
/// bandwidth heuristic. Returns [`BANDWIDTH_FLOOR`] when fewer than two points
/// or all points coincide.
///
/// # Panics
///
/// Does not panic: the internal sort uses a total order over finite distances.
#[must_use]
pub fn median_heuristic_bandwidth(points: &[Vec<f32>]) -> f32 {
    let mut dists: Vec<f32> = Vec::new();
    for (i, a) in points.iter().enumerate() {
        for b in points.iter().skip(i + 1) {
            dists.push(sq_dist(a, b).sqrt());
        }
    }
    if dists.is_empty() {
        return BANDWIDTH_FLOOR;
    }
    dists.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = dists.len() / 2;
    let median = if dists.len() % 2 == 0 {
        // Average the two central order statistics.
        f32::midpoint(
            dists.get(mid - 1).copied().unwrap_or(0.0),
            dists.get(mid).copied().unwrap_or(0.0),
        )
    } else {
        dists.get(mid).copied().unwrap_or(0.0)
    };
    median.max(BANDWIDTH_FLOOR)
}

/// Build the dense kernel (Gram) matrix over `points`, row-major `N × N`.
fn gram_matrix(points: &[Vec<f32>], kernel: Kernel) -> Vec<f32> {
    let n = points.len();
    let mut gram = vec![0.0_f32; n * n];
    let bandwidth = match kernel {
        Kernel::Rbf { bandwidth } if bandwidth.is_finite() && bandwidth > 0.0 => bandwidth,
        Kernel::Rbf { .. } => median_heuristic_bandwidth(points),
        Kernel::Energy => 0.0,
    };
    let two_sigma_sq = 2.0 * bandwidth * bandwidth;
    for (i, a) in points.iter().enumerate() {
        for (j, b) in points.iter().enumerate() {
            let d2 = sq_dist(a, b);
            let k = match kernel {
                Kernel::Rbf { .. } => (-d2 / two_sigma_sq).exp(),
                Kernel::Energy => -d2.sqrt(),
            };
            if let Some(slot) = gram.get_mut(i * n + j) {
                *slot = k;
            }
        }
    }
    gram
}

/// Compute the two-sample statistic from a pre-built Gram matrix, given an
/// index permutation `perm` whose first `m` entries are group X and the rest
/// group Y.
///
/// `unbiased` excludes diagonal (self) terms — required for the unbiased MMD²
/// estimator; the energy distance uses the biased form (its diagonal terms are
/// zero anyway, since `k(a, a) = 0`).
fn statistic_from_gram(gram: &[f32], n: usize, perm: &[usize], m: usize, unbiased: bool) -> f32 {
    let total = perm.len();
    let nn = total - m;
    let at = |a: usize, b: usize| -> f32 {
        let (ia, ib) = (
            perm.get(a).copied().unwrap_or(0),
            perm.get(b).copied().unwrap_or(0),
        );
        gram.get(ia * n + ib).copied().unwrap_or(0.0)
    };

    let mut within_first = 0.0;
    for a in 0..m {
        for b in 0..m {
            if unbiased && a == b {
                continue;
            }
            within_first += at(a, b);
        }
    }
    let mut within_second = 0.0;
    for a in m..total {
        for b in m..total {
            if unbiased && a == b {
                continue;
            }
            within_second += at(a, b);
        }
    }
    let mut across = 0.0;
    for a in 0..m {
        for b in m..total {
            across += at(a, b);
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let group_first = m as f32;
    #[allow(clippy::cast_precision_loss)]
    let group_second = nn as f32;
    let denom_first = if unbiased {
        group_first * (group_first - 1.0)
    } else {
        group_first * group_first
    };
    let denom_second = if unbiased {
        group_second * (group_second - 1.0)
    } else {
        group_second * group_second
    };
    within_first / denom_first + within_second / denom_second
        - 2.0 * across / (group_first * group_second)
}

/// Validate two samples share a common, non-zero dimension and meet the minimum
/// group size, returning that dimension.
fn validate(x: &[Vec<f32>], y: &[Vec<f32>]) -> Result<usize> {
    if x.len() < MIN_SAMPLES_PER_GROUP || y.len() < MIN_SAMPLES_PER_GROUP {
        return Err(ModelSentryError::Provider {
            message: format!(
                "two-sample test needs ≥{MIN_SAMPLES_PER_GROUP} samples per group; got \
                 {} baseline, {} run",
                x.len(),
                y.len()
            ),
        });
    }
    let dim = x.first().map_or(0, Vec::len);
    if dim == 0 {
        return Err(ModelSentryError::EmptyEmbedding);
    }
    for v in x.iter().chain(y.iter()) {
        if v.len() != dim {
            return Err(ModelSentryError::DimensionMismatch {
                expected: dim,
                actual: v.len(),
            });
        }
    }
    Ok(dim)
}

/// Unbiased squared MMD between `x` and `y` using an RBF kernel of the given
/// `bandwidth` (≤ 0 or non-finite ⇒ pooled median heuristic).
///
/// # Errors
///
/// [`ModelSentryError`] if either group has `< 2` samples or dimensions differ.
pub fn mmd2_unbiased(x: &[Vec<f32>], y: &[Vec<f32>], bandwidth: f32) -> Result<f32> {
    validate(x, y)?;
    let pooled: Vec<Vec<f32>> = x.iter().chain(y.iter()).cloned().collect();
    let gram = gram_matrix(&pooled, Kernel::Rbf { bandwidth });
    let perm: Vec<usize> = (0..pooled.len()).collect();
    Ok(statistic_from_gram(
        &gram,
        pooled.len(),
        &perm,
        x.len(),
        true,
    ))
}

/// Energy distance between `x` and `y` (parameter-free).
///
/// # Errors
///
/// [`ModelSentryError`] if either group has `< 2` samples or dimensions differ.
pub fn energy_distance(x: &[Vec<f32>], y: &[Vec<f32>]) -> Result<f32> {
    validate(x, y)?;
    let pooled: Vec<Vec<f32>> = x.iter().chain(y.iter()).cloned().collect();
    let gram = gram_matrix(&pooled, Kernel::Energy);
    let perm: Vec<usize> = (0..pooled.len()).collect();
    Ok(statistic_from_gram(
        &gram,
        pooled.len(),
        &perm,
        x.len(),
        false,
    ))
}

/// Run a permutation two-sample test of `baseline` vs `run`.
///
/// Builds the pooled Gram matrix once, then relabels the points `n_permutations`
/// times to form the null distribution of the statistic. The returned p-value is
/// `(1 + #{perm ≥ observed}) / (n_permutations + 1)` — the standard Monte-Carlo
/// permutation estimate, calibrated so `p < α` ⇒ FPR ≈ `α`.
///
/// `seed` makes the test deterministic (reproducible alerts and tests).
///
/// # Errors
///
/// [`ModelSentryError`] if either group has `< 2` samples or dimensions differ.
pub fn two_sample_test(
    baseline: &[Vec<f32>],
    run: &[Vec<f32>],
    kernel: Kernel,
    n_permutations: usize,
    seed: u64,
) -> Result<TwoSampleOutcome> {
    validate(baseline, run)?;
    let m = baseline.len();
    let pooled: Vec<Vec<f32>> = baseline.iter().chain(run.iter()).cloned().collect();
    let total = pooled.len();
    let unbiased = matches!(kernel, Kernel::Rbf { .. });

    let gram = gram_matrix(&pooled, kernel);
    let identity: Vec<usize> = (0..total).collect();
    let observed = statistic_from_gram(&gram, total, &identity, m, unbiased);

    let mut rng = SplitMix64::new(seed);
    let mut perm = identity;
    let mut at_least = 0usize;
    for _ in 0..n_permutations {
        fisher_yates(&mut perm, &mut rng);
        let stat = statistic_from_gram(&gram, total, &perm, m, unbiased);
        if stat >= observed - PERMUTATION_TOLERANCE {
            at_least += 1;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let p_value = (1.0 + at_least as f32) / (1.0 + n_permutations as f32);
    Ok(TwoSampleOutcome {
        statistic: observed,
        p_value,
        n_baseline: m,
        n_run: run.len(),
        n_permutations,
    })
}

/// In-place Fisher–Yates shuffle using the supplied PRNG.
fn fisher_yates(slice: &mut [usize], rng: &mut SplitMix64) {
    let len = slice.len();
    for i in (1..len).rev() {
        let j = rng.next_bounded(i + 1);
        slice.swap(i, j);
    }
}

/// `SplitMix64` — a tiny, fast, dependency-free PRNG used to keep the
/// permutation test deterministic and reproducible without pulling in `rand`.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in `[0, bound)` via Lemire's multiply-shift (bias is
    /// negligible for the small bounds used here).
    fn next_bounded(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        let product = u128::from(self.next_u64()) * bound as u128;
        #[allow(clippy::cast_possible_truncation)]
        let result = (product >> 64) as usize;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build `count` points in `dim` dimensions on a deterministic lattice,
    /// translated by `shift` on the first axis.
    fn cluster(count: usize, dim: usize, shift: f32, jitter: f32) -> Vec<Vec<f32>> {
        (0..count)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let base = i as f32 * jitter;
                let mut v = vec![base; dim];
                if let Some(first) = v.first_mut() {
                    *first += shift;
                }
                v
            })
            .collect()
    }

    #[test]
    fn errors_on_too_few_samples() {
        let x = vec![vec![1.0, 2.0]];
        let y = cluster(5, 2, 0.0, 0.1);
        assert!(energy_distance(&x, &y).is_err());
        assert!(mmd2_unbiased(&x, &y, 1.0).is_err());
        assert!(two_sample_test(&x, &y, Kernel::Energy, 50, 1).is_err());
    }

    #[test]
    fn errors_on_dimension_mismatch() {
        let x = vec![vec![1.0, 2.0], vec![1.0, 2.1]];
        let y = vec![vec![1.0, 2.0, 3.0], vec![1.0, 2.0, 3.1]];
        assert!(energy_distance(&x, &y).is_err());
    }

    #[test]
    fn energy_distance_is_zero_for_identical_samples() {
        let x = cluster(8, 4, 0.0, 0.3);
        let e = energy_distance(&x, &x).unwrap();
        assert!(
            e.abs() < 1e-3,
            "expected ~0 energy for identical sets, got {e}"
        );
    }

    #[test]
    fn energy_distance_grows_with_separation() {
        let base = cluster(10, 4, 0.0, 0.2);
        let near = cluster(10, 4, 0.5, 0.2);
        let far = cluster(10, 4, 5.0, 0.2);
        let e_near = energy_distance(&base, &near).unwrap();
        let e_far = energy_distance(&base, &far).unwrap();
        assert!(
            e_far > e_near,
            "energy should grow with separation: {e_near} vs {e_far}"
        );
    }

    #[test]
    fn mmd2_is_small_for_same_distribution_and_large_for_shift() {
        let a = cluster(12, 6, 0.0, 0.25);
        let b = cluster(12, 6, 0.0, 0.25);
        let shifted = cluster(12, 6, 4.0, 0.25);
        let same = mmd2_unbiased(&a, &b, f32::NAN).unwrap();
        let diff = mmd2_unbiased(&a, &shifted, f32::NAN).unwrap();
        assert!(
            diff > same,
            "MMD² should be larger under a shift: same={same}, diff={diff}"
        );
    }

    #[test]
    fn permutation_pvalue_is_small_under_strong_drift() {
        let base = cluster(15, 5, 0.0, 0.15);
        let drifted = cluster(15, 5, 6.0, 0.15);
        let out = two_sample_test(&base, &drifted, Kernel::rbf_median(), 200, 42);
        let out = out.unwrap();
        assert!(
            out.p_value < 0.05,
            "strong drift should be significant, got p={}",
            out.p_value
        );
        assert!((0.0..=1.0).contains(&out.p_value));
    }

    #[test]
    fn permutation_pvalue_is_not_significant_under_no_drift() {
        // Two interleaved samples from the same lattice — no real drift.
        let a = cluster(20, 5, 0.0, 0.2);
        let b = cluster(20, 5, 0.0, 0.2);
        let out = two_sample_test(&a, &b, Kernel::Energy, 200, 7).unwrap();
        assert!(
            out.p_value > 0.05,
            "identical distributions should not alert, got p={}",
            out.p_value
        );
    }

    #[test]
    fn permutation_test_is_deterministic_for_a_fixed_seed() {
        let a = cluster(10, 4, 0.0, 0.3);
        let b = cluster(10, 4, 1.0, 0.3);
        let p1 = two_sample_test(&a, &b, Kernel::rbf_median(), 100, 123)
            .unwrap()
            .p_value;
        let p2 = two_sample_test(&a, &b, Kernel::rbf_median(), 100, 123)
            .unwrap()
            .p_value;
        assert!(
            (p1 - p2).abs() < f32::EPSILON,
            "same seed must give same p-value"
        );
    }

    #[test]
    fn statistic_is_symmetric_in_its_arguments() {
        let a = cluster(8, 4, 0.0, 0.2);
        let b = cluster(8, 4, 1.5, 0.2);
        let e_ab = energy_distance(&a, &b).unwrap();
        let e_ba = energy_distance(&b, &a).unwrap();
        assert!(
            (e_ab - e_ba).abs() < 1e-4,
            "energy distance must be symmetric"
        );
    }

    #[test]
    fn median_heuristic_is_positive() {
        let pts = cluster(6, 3, 0.0, 0.5);
        assert!(median_heuristic_bandwidth(&pts) > 0.0);
        // Degenerate (all identical) falls back to the floor, not zero.
        let same = vec![vec![1.0, 1.0]; 5];
        assert!(median_heuristic_bandwidth(&same) >= BANDWIDTH_FLOOR);
    }
}
