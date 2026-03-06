//! Shannon entropy over token distributions and entropy delta computation.

#![allow(clippy::module_name_repetitions)]

use std::collections::HashMap;

use modelsentry_common::error::{ModelSentryError, Result};

/// Shannon entropy H(X) = −Σ p(x) log₂ p(x) for a token sequence.
///
/// Tokens are counted by frequency; the resulting distribution is used to
/// compute entropy in **bits**. Returns `0.0` for a single unique token.
#[must_use]
pub fn token_entropy(tokens: &[String]) -> f32 {
    if tokens.is_empty() {
        return 0.0;
    }
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for t in tokens {
        *counts.entry(t.as_str()).or_insert(0) += 1;
    }
    #[allow(clippy::cast_precision_loss)]
    let n = tokens.len() as f32;
    counts
        .values()
        .map(|&c| {
            #[allow(clippy::cast_precision_loss)]
            let p = c as f32 / n;
            if p > 0.0 {
                -p * p.log2()
            } else {
                0.0
            }
        })
        .sum()
}

/// Mean entropy across a slice of tokenised completions.
///
/// # Errors
///
/// [`ModelSentryError::Provider`] if `completions` is empty.
pub fn mean_entropy(completions: &[Vec<String>]) -> Result<f32> {
    if completions.is_empty() {
        return Err(ModelSentryError::Provider {
            message: "cannot compute mean entropy of empty completions list".into(),
        });
    }
    #[allow(clippy::cast_precision_loss)]
    let mean = completions.iter().map(|toks| token_entropy(toks)).sum::<f32>()
        / completions.len() as f32;
    Ok(mean)
}

/// Absolute difference in mean entropy between run completions and baseline tokens.
///
/// `|H(run) − H(baseline)|`
///
/// # Errors
///
/// Propagates errors from [`mean_entropy`] (empty slice).
pub fn entropy_delta(
    run_completions: &[Vec<String>],
    baseline_completions: &[Vec<String>],
) -> Result<f32> {
    let h_run = mean_entropy(run_completions)?;
    let h_baseline = mean_entropy(baseline_completions)?;
    Ok((h_run - h_baseline).abs())
}

/// Whitespace tokenizer: split on whitespace, lowercase, strip leading/trailing
/// ASCII punctuation from each token.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.to_lowercase()
                .trim_matches(|c: char| c.is_ascii_punctuation())
                .to_owned()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5_f32;

    #[test]
    fn entropy_of_single_token_is_zero() {
        let tokens = vec!["hello".to_owned()];
        assert!(token_entropy(&tokens).abs() < EPS);
    }

    #[test]
    fn entropy_of_two_different_tokens_is_one_bit() {
        // Two unique tokens, each appearing once → p = 0.5 each → H = 1 bit
        let tokens = vec!["hello".to_owned(), "world".to_owned()];
        let h = token_entropy(&tokens);
        assert!((h - 1.0).abs() < EPS, "expected 1 bit, got {h}");
    }

    #[test]
    fn entropy_is_nonnegative() {
        let tokens = vec!["a".to_owned(), "b".to_owned(), "a".to_owned()];
        assert!(token_entropy(&tokens) >= 0.0);
    }

    #[test]
    fn entropy_of_empty_tokens_is_zero() {
        assert!(token_entropy(&[]).abs() < EPS);
    }

    #[test]
    fn entropy_delta_same_completions_is_zero() {
        let completions = vec![
            vec!["hello".to_owned(), "world".to_owned()],
            vec!["foo".to_owned(), "bar".to_owned()],
        ];
        let delta = entropy_delta(&completions, &completions).unwrap();
        assert!(delta.abs() < EPS, "expected 0, got {delta}");
    }

    #[test]
    fn mean_entropy_rejects_empty_input() {
        assert!(mean_entropy(&[]).is_err());
    }

    /// 4 equally likely tokens → H = log₂(4) = 2 bits.
    #[test]
    fn entropy_of_four_uniform_tokens_is_two_bits() {
        let tokens = vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned()];
        let h = token_entropy(&tokens);
        assert!((h - 2.0).abs() < EPS, "expected 2 bits, got {h}");
    }

    /// A uniform distribution has strictly higher entropy than a skewed one.
    #[test]
    fn uniform_distribution_higher_entropy_than_skewed() {
        // Uniform: equal representation
        let uniform = vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned()];
        // Skewed: mostly one token
        let skewed = vec!["a".to_owned(), "a".to_owned(), "a".to_owned(), "b".to_owned()];
        assert!(
            token_entropy(&uniform) > token_entropy(&skewed),
            "uniform should have higher entropy than skewed"
        );
    }

    /// entropy_delta is independent of which side is higher (absolute value).
    #[test]
    fn entropy_delta_is_symmetric() {
        let high_entropy = vec![vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned()]];
        let low_entropy = vec![vec!["a".to_owned(), "a".to_owned(), "a".to_owned(), "b".to_owned()]];
        let d1 = entropy_delta(&high_entropy, &low_entropy).unwrap();
        let d2 = entropy_delta(&low_entropy, &high_entropy).unwrap();
        assert!((d1 - d2).abs() < EPS, "entropy_delta should be symmetric: {d1} vs {d2}");
    }

    #[test]
    fn tokenize_lowercases_and_strips_punctuation() {
        let tokens = tokenize("Hello, World!");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn tokenize_handles_empty_string() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn tokenize_handles_multiple_spaces() {
        let tokens = tokenize("  foo   bar  ");
        assert_eq!(tokens, vec!["foo", "bar"]);
    }
}
