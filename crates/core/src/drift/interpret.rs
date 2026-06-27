//! Human-readable interpretation of a drift assessment.
//!
//! This reports the *statistical verdict* honestly — which test ran, the
//! calibrated p-value, and the prompt that moved most — rather than guessing at
//! the semantic meaning of the change. (True semantic explanation would require
//! a separate LLM-as-judge pass over the answer text.)

use super::assessment::{DriftAssessment, METHOD_PER_PROMPT};
use modelsentry_common::models::DriftLevel;

/// Build a one-paragraph interpretation of `assessment`, judged against
/// `target_fpr`.
#[must_use]
pub fn interpret(assessment: &DriftAssessment, target_fpr: f32) -> String {
    let method = if assessment.method == METHOD_PER_PROMPT {
        "per-prompt conformal"
    } else {
        "pooled MMD/energy"
    };
    let p = assessment.combined_p_value;

    if assessment.level == DriftLevel::None {
        return format!(
            "No drift detected: the run's outputs are statistically consistent with the baseline \
             ({method} test, combined p = {p:.4} ≥ target FPR {target_fpr:.4})."
        );
    }

    let lead = match assessment.level {
        DriftLevel::Low => "Low drift",
        DriftLevel::Medium => "Medium drift",
        DriftLevel::High => "High drift",
        DriftLevel::Critical => "Critical drift",
        DriftLevel::None => "Drift",
    };

    // Identify the strongest per-prompt signal, if available.
    let strongest = assessment
        .per_prompt
        .iter()
        .min_by(|a, b| {
            a.p_value
                .partial_cmp(&b.p_value)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|pp| {
            format!(
                " Strongest signal: prompt #{} (p = {:.4}, baseline n = {}).",
                pp.prompt_index, pp.p_value, pp.n_baseline
            )
        })
        .unwrap_or_default();

    format!(
        "{lead}: the {method} test rejects the no-drift hypothesis (combined p = {p:.4} < target \
         FPR {target_fpr:.4}), so the model's outputs have shifted relative to the baseline.{strongest}"
    )
}

#[cfg(test)]
mod tests {
    use super::super::assessment::{
        DriftAssessment, METHOD_PER_PROMPT, METHOD_POOLED, PromptDrift,
    };
    use super::*;

    fn assessment(level: DriftLevel, p: f32, method: &'static str) -> DriftAssessment {
        DriftAssessment {
            combined_p_value: p,
            statistic: -(p.max(f32::MIN_POSITIVE)).log10(),
            level,
            method,
            per_prompt: vec![PromptDrift {
                prompt_index: 2,
                p_value: p,
                n_baseline: 40,
            }],
        }
    }

    #[test]
    fn none_is_reported_as_consistent() {
        let text = interpret(&assessment(DriftLevel::None, 0.4, METHOD_PER_PROMPT), 0.01);
        assert!(text.contains("No drift detected"), "{text}");
        assert!(text.contains("consistent"), "{text}");
    }

    #[test]
    fn drift_names_method_and_strongest_prompt() {
        let text = interpret(
            &assessment(DriftLevel::High, 0.0001, METHOD_PER_PROMPT),
            0.01,
        );
        assert!(text.contains("High drift"), "{text}");
        assert!(text.contains("per-prompt conformal"), "{text}");
        assert!(text.contains("prompt #2"), "{text}");
    }

    #[test]
    fn pooled_method_is_named() {
        let mut a = assessment(DriftLevel::Medium, 0.001, METHOD_POOLED);
        a.per_prompt.clear();
        let text = interpret(&a, 0.01);
        assert!(text.contains("pooled MMD/energy"), "{text}");
    }
}
