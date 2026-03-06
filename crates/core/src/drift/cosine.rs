//! Cosine similarity and cosine distance between embeddings.

#![allow(clippy::module_name_repetitions)]

use super::Embedding;
use modelsentry_common::error::{ModelSentryError, Result};

/// Cosine similarity between two embeddings, in the range `[−1, 1]`.
///
/// # Errors
///
/// - [`ModelSentryError::DimensionMismatch`] if dimensions differ.
/// - [`ModelSentryError::Provider`] if either embedding has zero L2 norm.
pub fn cosine_similarity(a: &Embedding, b: &Embedding) -> Result<f32> {
    let norm_a = a.l2_norm();
    let norm_b = b.l2_norm();
    if norm_a <= 0.0 {
        return Err(ModelSentryError::Provider {
            message: "embedding `a` has zero L2 norm — cosine similarity undefined".into(),
        });
    }
    if norm_b <= 0.0 {
        return Err(ModelSentryError::Provider {
            message: "embedding `b` has zero L2 norm — cosine similarity undefined".into(),
        });
    }
    let dot = a.dot(b)?;
    Ok(dot / (norm_a * norm_b))
}

/// Cosine distance between two embeddings, normalised to `[0, 1]`.
///
/// Defined as `(1 − cosine_similarity(a, b)) / 2`, so:
/// - identical direction → 0.0
/// - orthogonal → 0.5
/// - opposite direction → 1.0
///
/// # Errors
///
/// Propagates errors from [`cosine_similarity`].
pub fn cosine_distance(a: &Embedding, b: &Embedding) -> Result<f32> {
    let sim = cosine_similarity(a, b)?;
    // Clamp to [-1, 1] before computing distance to counter floating-point drift.
    let sim_clamped = sim.clamp(-1.0, 1.0);
    Ok((1.0 - sim_clamped) / 2.0)
}

/// Cosine distance from a single embedding to the centroid of a set.
///
/// Equivalent to `cosine_distance(embedding, &Embedding::centroid(set))`.
///
/// # Errors
///
/// Propagates errors from [`Embedding::centroid`] and [`cosine_distance`].
pub fn distance_to_centroid(embedding: &Embedding, set: &[Embedding]) -> Result<f32> {
    let centroid = Embedding::centroid(set)?;
    cosine_distance(embedding, &centroid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const EPS: f32 = 1e-5_f32;

    fn make(vals: Vec<f32>) -> Embedding {
        Embedding::new(vals).unwrap()
    }

    #[test]
    fn cosine_same_vector_is_zero_distance() {
        let v = make(vec![1.0, 2.0, 3.0]);
        let d = cosine_distance(&v, &v).unwrap();
        assert!(d.abs() < EPS, "expected 0, got {d}");
    }

    #[test]
    fn cosine_orthogonal_vectors_is_half_distance() {
        let a = make(vec![1.0, 0.0]);
        let b = make(vec![0.0, 1.0]);
        let d = cosine_distance(&a, &b).unwrap();
        assert!((d - 0.5).abs() < EPS, "expected 0.5, got {d}");
    }

    #[test]
    fn cosine_opposite_vectors_is_one_distance() {
        let a = make(vec![1.0, 0.0]);
        let b = make(vec![-1.0, 0.0]);
        let d = cosine_distance(&a, &b).unwrap();
        assert!((d - 1.0).abs() < EPS, "expected 1.0, got {d}");
    }

    #[test]
    fn cosine_rejects_zero_norm_vector() {
        let a = make(vec![1.0, 0.0]);
        // Embedding::new accepts 0.0 (finite), so we can create a zero-norm vector
        let z = Embedding::new(vec![0.0, 0.0]).unwrap();
        assert!(cosine_similarity(&a, &z).is_err());
        assert!(cosine_similarity(&z, &a).is_err());
    }

    #[test]
    fn cosine_rejects_mismatched_dims() {
        let a = make(vec![1.0, 0.0]);
        let b = make(vec![1.0, 0.0, 0.0]);
        assert!(cosine_distance(&a, &b).is_err());
    }

    #[test]
    fn distance_to_centroid_of_identical_set_is_zero() {
        let v = make(vec![1.0, 2.0, 3.0]);
        let set = vec![v.clone(), v.clone(), v.clone()];
        let d = distance_to_centroid(&v, &set).unwrap();
        assert!(d.abs() < EPS, "expected 0, got {d}");
    }

    proptest! {
        #[test]
        fn cosine_distance_is_in_unit_interval(
            (a_vals, b_vals) in (2_usize..=8_usize).prop_flat_map(|len| {
                (
                    proptest::collection::vec(0.001_f32..10.0_f32, len),
                    proptest::collection::vec(0.001_f32..10.0_f32, len),
                )
            }),
        ) {
            let a = Embedding::new(a_vals).unwrap();
            let b = Embedding::new(b_vals).unwrap();
            if let Ok(d) = cosine_distance(&a, &b) {
                prop_assert!(d >= -EPS, "distance below 0: {d}");
                prop_assert!(d <= 1.0 + EPS, "distance above 1: {d}");
            }
        }

        #[test]
        fn cosine_distance_is_symmetric(
            (a_vals, b_vals) in (2_usize..=8_usize).prop_flat_map(|len| {
                (
                    proptest::collection::vec(0.001_f32..10.0_f32, len),
                    proptest::collection::vec(0.001_f32..10.0_f32, len),
                )
            }),
        ) {
            let a = Embedding::new(a_vals).unwrap();
            let b = Embedding::new(b_vals).unwrap();
            if let (Ok(d_ab), Ok(d_ba)) = (cosine_distance(&a, &b), cosine_distance(&b, &a)) {
                prop_assert!((d_ab - d_ba).abs() < EPS, "asymmetric: d(a,b)={d_ab} d(b,a)={d_ba}");
            }
        }
    }
}
