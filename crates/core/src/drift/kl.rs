//! KL divergence and distribution normalization utilities.

#![allow(clippy::module_name_repetitions)]

use modelsentry_common::error::{ModelSentryError, Result};

/// KL divergence `D_KL(p ‖ q)` for discrete probability distributions.
///
/// Both slices must have the same length, all values must be ≥ 0, and
/// `q[i]` must not be zero where `p[i] > 0` (undefined in continuous KL).
/// When `p[i]` is zero the term contributes zero (0 · log(0/q) = 0 by convention).
///
/// # Errors
///
/// - [`ModelSentryError::Provider`] if either slice is empty, any value is
///   negative, or `q[i] == 0` where `p[i] > 0`.
/// - [`ModelSentryError::DimensionMismatch`] if the slices have different lengths.
pub fn kl_divergence(p: &[f32], q: &[f32]) -> Result<f32> {
    if p.is_empty() || q.is_empty() {
        return Err(ModelSentryError::Provider {
            message: "distribution must be non-empty".into(),
        });
    }
    if p.len() != q.len() {
        return Err(ModelSentryError::DimensionMismatch {
            expected: p.len(),
            actual: q.len(),
        });
    }
    for (i, (&pi, &qi)) in p.iter().zip(q.iter()).enumerate() {
        if pi < 0.0 || qi < 0.0 {
            return Err(ModelSentryError::Provider {
                message: format!("distribution values must be non-negative; got p[{i}]={pi}, q[{i}]={qi}"),
            });
        }
        if pi > 0.0 && qi <= 0.0 {
            return Err(ModelSentryError::Provider {
                message: format!("q[{i}] is zero where p[{i}] is non-zero — KL divergence is undefined"),
            });
        }
    }
    let kl: f32 = p
        .iter()
        .zip(q.iter())
        .filter(|&(&pi, _)| pi > 0.0)
        .map(|(&pi, &qi)| pi * (pi / qi).ln())
        .sum();
    Ok(kl)
}

/// KL divergence between two univariate Gaussians N(μ₁, σ₁²) ‖ N(μ₂, σ₂²).
///
/// Closed-form: `ln(σ₂/σ₁) + (σ₁² + (μ₁ − μ₂)²) / (2σ₂²) − ½`
///
/// # Errors
///
/// [`ModelSentryError::Provider`] if either sigma is non-positive.
pub fn gaussian_kl(mu1: f32, sigma1: f32, mu2: f32, sigma2: f32) -> Result<f32> {
    if sigma1 <= 0.0 || sigma2 <= 0.0 {
        return Err(ModelSentryError::Provider {
            message: format!(
                "sigma must be positive; got sigma1={sigma1}, sigma2={sigma2}"
            ),
        });
    }
    let kl = (sigma2 / sigma1).ln()
        + (sigma1 * sigma1 + (mu1 - mu2) * (mu1 - mu2)) / (2.0 * sigma2 * sigma2)
        - 0.5;
    Ok(kl)
}

/// Normalize a non-negative frequency count vector into a probability distribution
/// (values sum to 1.0).
///
/// # Errors
///
/// [`ModelSentryError::Provider`] if the input is empty, all counts are zero,
/// or any count is negative.
pub fn normalize(counts: &[f32]) -> Result<Vec<f32>> {
    if counts.is_empty() {
        return Err(ModelSentryError::Provider {
            message: "cannot normalize empty counts".into(),
        });
    }
    if counts.iter().any(|c| *c < 0.0) {
        return Err(ModelSentryError::Provider {
            message: "counts must be non-negative".into(),
        });
    }
    let sum: f32 = counts.iter().sum();
    if sum <= 0.0 {
        return Err(ModelSentryError::Provider {
            message: "cannot normalize all-zero counts".into(),
        });
    }
    Ok(counts.iter().map(|c| c / sum).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const EPS: f32 = 1e-5_f32;

    #[test]
    fn kl_same_distribution_is_zero() {
        let p = vec![0.25, 0.25, 0.25, 0.25];
        let result = kl_divergence(&p, &p).unwrap();
        assert!(result.abs() < EPS, "expected ~0, got {result}");
    }

    #[test]
    fn kl_is_nonnegative() {
        let p = vec![0.5, 0.5];
        let q = vec![0.8, 0.2];
        let result = kl_divergence(&p, &q).unwrap();
        assert!(result >= 0.0, "KL must be non-negative, got {result}");
    }

    #[test]
    fn kl_rejects_mismatched_lengths() {
        assert!(kl_divergence(&[0.5, 0.5], &[0.5, 0.3, 0.2]).is_err());
    }

    #[test]
    fn kl_rejects_empty_input() {
        assert!(kl_divergence(&[], &[]).is_err());
    }

    #[test]
    fn kl_rejects_q_zero_where_p_nonzero() {
        let p = vec![0.5, 0.5];
        let q = vec![1.0, 0.0];
        assert!(kl_divergence(&p, &q).is_err());
    }

    #[test]
    fn gaussian_kl_same_params_is_zero() {
        let result = gaussian_kl(1.0, 0.5, 1.0, 0.5).unwrap();
        assert!(result.abs() < EPS, "expected ~0, got {result}");
    }

    #[test]
    fn gaussian_kl_rejects_zero_sigma() {
        assert!(gaussian_kl(0.0, 0.0, 0.0, 1.0).is_err());
        assert!(gaussian_kl(0.0, 1.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn normalize_sums_to_one() {
        let counts = vec![1.0, 2.0, 3.0, 4.0];
        let dist = normalize(&counts).unwrap();
        let sum: f32 = dist.iter().sum();
        assert!((sum - 1.0).abs() < EPS, "expected sum=1, got {sum}");
    }

    #[test]
    fn normalize_rejects_all_zero_input() {
        assert!(normalize(&[0.0, 0.0, 0.0]).is_err());
    }

    /// `D_KL(p||q)` for p=[0.5,0.5], q=[0.75,0.25].
    /// Hand-computed: 0.5*ln(2/3) + 0.5*ln(2) ≈ 0.1438
    #[test]
    fn kl_known_value() {
        let p = vec![0.5_f32, 0.5];
        let q = vec![0.75_f32, 0.25];
        let result = kl_divergence(&p, &q).unwrap();
        assert!(
            (result - 0.143_841_2_f32).abs() < 1e-4,
            "expected ≈0.1438, got {result}"
        );
    }

    /// KL divergence is NOT symmetric: `D_KL(p||q)` ≠ `D_KL(q||p)`.
    /// Catches any accidentally symmetrised implementation.
    #[test]
    fn kl_is_asymmetric() {
        let p = vec![0.9_f32, 0.1];
        let q = vec![0.5_f32, 0.5];
        let kl_pq = kl_divergence(&p, &q).unwrap();
        let kl_qp = kl_divergence(&q, &p).unwrap();
        assert!(
            (kl_pq - kl_qp).abs() > 0.01,
            "KL should be asymmetric: D(p||q)={kl_pq}, D(q||p)={kl_qp}"
        );
    }

    /// For N(0,1) || N(0,2): KL = ln(2) + 1/8 - 0.5 ≈ 0.3181
    #[test]
    fn gaussian_kl_known_value() {
        let result = gaussian_kl(0.0, 1.0, 0.0, 2.0).unwrap();
        assert!(
            (result - 0.318_147_2_f32).abs() < 1e-4,
            "expected ≈0.3181, got {result}"
        );
    }

    /// Gaussian KL is NOT symmetric.
    #[test]
    fn gaussian_kl_is_asymmetric() {
        let kl_12 = gaussian_kl(0.0, 1.0, 0.0, 2.0).unwrap();
        let kl_21 = gaussian_kl(0.0, 2.0, 0.0, 1.0).unwrap();
        assert!(
            (kl_12 - kl_21).abs() > 0.01,
            "Gaussian KL should be asymmetric: D(1||2)={kl_12}, D(2||1)={kl_21}"
        );
    }

    /// Normalizing preserves relative ratios between components.
    #[test]
    fn normalize_preserves_ratios() {
        let counts = vec![1.0_f32, 2.0, 3.0];
        let dist = normalize(&counts).unwrap();
        // 1:2:3 ratios must be preserved
        assert!((dist[1] / dist[0] - 2.0).abs() < 1e-5);
        assert!((dist[2] / dist[0] - 3.0).abs() < 1e-5);
    }

    proptest! {
        #[test]
        fn kl_divergence_is_always_nonnegative(
            vals in proptest::collection::vec(0.001_f32..1.0_f32, 2_usize..=10),
        ) {
            // Create two valid distributions from the same length
            let sum_p: f32 = vals.iter().sum();
            let p: Vec<f32> = vals.iter().map(|v| v / sum_p).collect();
            // Shift slightly to form q
            let q_raw: Vec<f32> = vals.iter().map(|v| v + 0.01).collect();
            let sum_q: f32 = q_raw.iter().sum();
            let q: Vec<f32> = q_raw.iter().map(|v| v / sum_q).collect();
            if let Ok(kl) = kl_divergence(&p, &q) {
                prop_assert!(kl >= -EPS, "KL must be non-negative, got {kl}");
            }
        }

        #[test]
        fn kl_with_uniform_q_is_finite(
            vals in proptest::collection::vec(0.001_f32..1.0_f32, 2_usize..=10),
        ) {
            let n = vals.len();
            let sum: f32 = vals.iter().sum();
            let p: Vec<f32> = vals.iter().map(|v| v / sum).collect();
            #[allow(clippy::cast_precision_loss)]
            let uniform = vec![1.0_f32 / n as f32; n];
            if let Ok(kl) = kl_divergence(&p, &uniform) {
                prop_assert!(kl.is_finite(), "KL must be finite, got {kl}");
                prop_assert!(kl >= -EPS);
            }
        }
    }
}
