//! Drift algorithm primitives.
//!
//! Sub-modules implement the individual algorithms; [`Embedding`] is the shared
//! input type used by all of them.

pub mod calculator;
pub mod cosine;
pub mod entropy;
pub mod kl;

use modelsentry_common::error::{ModelSentryError, Result};

/// A validated, non-empty embedding vector.
///
/// All values are guaranteed to be finite (no NaN or Inf). Constructors enforce
/// this invariant so consumers can safely perform arithmetic on the inner slice.
#[derive(Debug, Clone)]
pub struct Embedding(Vec<f32>);

impl Embedding {
    /// Construct a new [`Embedding`], validating that the vector is non-empty
    /// and all values are finite.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::EmptyEmbedding`] if `raw` is empty.
    /// - [`ModelSentryError::Provider`] if any value is NaN or infinite.
    pub fn new(raw: Vec<f32>) -> Result<Self> {
        if raw.is_empty() {
            return Err(ModelSentryError::EmptyEmbedding);
        }
        if raw.iter().any(|v| !v.is_finite()) {
            return Err(ModelSentryError::Provider {
                message: "embedding contains non-finite values (NaN or Inf)".into(),
            });
        }
        Ok(Self(raw))
    }

    /// Number of dimensions.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.0.len()
    }

    /// View the underlying float values.
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// Compute the component-wise arithmetic mean (centroid) of a non-empty
    /// slice of embeddings. All embeddings must share the same dimension.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::EmptyEmbedding`] if the slice is empty.
    /// - [`ModelSentryError::DimensionMismatch`] if any embedding has a
    ///   different dimension than the first.
    pub fn centroid(embeddings: &[Self]) -> Result<Self> {
        let first = embeddings.first().ok_or(ModelSentryError::EmptyEmbedding)?;
        let dim = first.dim();
        for e in embeddings {
            if e.dim() != dim {
                return Err(ModelSentryError::DimensionMismatch {
                    expected: dim,
                    actual: e.dim(),
                });
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let n = embeddings.len() as f32;
        let mut sum = vec![0.0_f32; dim];
        for e in embeddings {
            for (s, v) in sum.iter_mut().zip(e.as_slice()) {
                *s += v;
            }
        }
        let centroid: Vec<f32> = sum.iter().map(|s| s / n).collect();
        Self::new(centroid)
    }

    /// Dot product with another embedding.
    ///
    /// # Errors
    ///
    /// [`ModelSentryError::DimensionMismatch`] if the dimensions differ.
    pub fn dot(&self, other: &Self) -> Result<f32> {
        if self.dim() != other.dim() {
            return Err(ModelSentryError::DimensionMismatch {
                expected: self.dim(),
                actual: other.dim(),
            });
        }
        Ok(self.0.iter().zip(other.0.iter()).map(|(a, b)| a * b).sum())
    }

    /// L2 (Euclidean) norm of this embedding.
    #[must_use]
    pub fn l2_norm(&self) -> f32 {
        self.0.iter().map(|v| v * v).sum::<f32>().sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn embedding_rejects_empty_vec() {
        assert!(Embedding::new(vec![]).is_err());
    }

    #[test]
    fn embedding_rejects_nan() {
        assert!(Embedding::new(vec![f32::NAN]).is_err());
    }

    #[test]
    fn embedding_rejects_infinity() {
        assert!(Embedding::new(vec![f32::INFINITY]).is_err());
        assert!(Embedding::new(vec![f32::NEG_INFINITY]).is_err());
    }

    #[test]
    fn centroid_of_one_is_itself() {
        let e = Embedding::new(vec![1.0, 2.0, 3.0]).unwrap();
        let c = Embedding::centroid(std::slice::from_ref(&e)).unwrap();
        assert_eq!(c.as_slice(), e.as_slice());
    }

    #[test]
    fn centroid_of_two_is_midpoint() {
        let a = Embedding::new(vec![0.0, 0.0]).unwrap();
        let b = Embedding::new(vec![2.0, 4.0]).unwrap();
        let c = Embedding::centroid(&[a, b]).unwrap();
        assert!((c.as_slice()[0] - 1.0).abs() < 1e-6);
        assert!((c.as_slice()[1] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn centroid_rejects_empty_slice() {
        assert!(Embedding::centroid(&[]).is_err());
    }

    #[test]
    fn centroid_rejects_mismatched_dims() {
        let a = Embedding::new(vec![1.0, 2.0]).unwrap();
        let b = Embedding::new(vec![1.0, 2.0, 3.0]).unwrap();
        assert!(Embedding::centroid(&[a, b]).is_err());
    }

    #[test]
    fn l2_norm_of_unit_vector_is_one() {
        let e = Embedding::new(vec![1.0, 0.0, 0.0]).unwrap();
        assert!((e.l2_norm() - 1.0).abs() < 1e-6);
    }

    /// [1,2,3]·[4,5,6] = 4+10+18 = 32.
    #[test]
    fn dot_product_known_value() {
        let a = Embedding::new(vec![1.0, 2.0, 3.0]).unwrap();
        let b = Embedding::new(vec![4.0, 5.0, 6.0]).unwrap();
        let result = a.dot(&b).unwrap();
        assert!((result - 32.0).abs() < 1e-5, "expected 32, got {result}");
    }

    /// Dot product of orthogonal vectors is zero.
    #[test]
    fn dot_product_of_orthogonal_vectors_is_zero() {
        let a = Embedding::new(vec![1.0, 0.0]).unwrap();
        let b = Embedding::new(vec![0.0, 1.0]).unwrap();
        let result = a.dot(&b).unwrap();
        assert!(result.abs() < 1e-5, "expected 0, got {result}");
    }

    /// Dot product rejects mismatched dimensions.
    #[test]
    fn dot_product_rejects_mismatched_dims() {
        let a = Embedding::new(vec![1.0, 0.0]).unwrap();
        let b = Embedding::new(vec![1.0, 0.0, 0.0]).unwrap();
        assert!(a.dot(&b).is_err());
    }

    proptest! {
        #[test]
        fn centroid_dim_equals_input_dim(
            vals in proptest::collection::vec(-1000.0_f32..1000.0_f32, 1_usize..=8),
            n_copies in 1_usize..=5,
        ) {
            let dim = vals.len();
            let e = Embedding::new(vals).unwrap();
            let embeddings: Vec<Embedding> = vec![e; n_copies];
            let centroid = Embedding::centroid(&embeddings).unwrap();
            prop_assert_eq!(centroid.dim(), dim);
        }

        #[test]
        fn l2_norm_is_nonnegative(
            vals in proptest::collection::vec(-1000.0_f32..1000.0_f32, 1_usize..=16),
        ) {
            let e = Embedding::new(vals).unwrap();
            prop_assert!(e.l2_norm() >= 0.0);
        }
    }
}
