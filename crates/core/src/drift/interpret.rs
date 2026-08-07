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
///
/// `model_change` is `Some((baseline_version, run_version))` when the provider
/// reported a different model version than the one pinned at baseline capture;
/// the verdict then names the change as an exchangeability caveat (the baseline
/// describes a different model), akin to the embedding-dimension guard.
#[must_use]
pub fn interpret(
    assessment: &DriftAssessment,
    target_fpr: f32,
    model_change: Option<(&str, &str)>,
) -> String {
    let method = if assessment.method == METHOD_PER_PROMPT {
        "per-prompt conformal"
    } else {
        "pooled MMD/energy"
    };
    let p = assessment.combined_p_value;

    // Honesty caveat: a near-constant baseline measures embedding noise, not
    // behaviour. Surface it whether or not the run "drifted".
    let degenerate: Vec<usize> = assessment
        .per_prompt
        .iter()
        .filter(|pp| pp.low_variance_baseline)
        .map(|pp| pp.prompt_index)
        .collect();
    let health_note = if degenerate.is_empty() {
        String::new()
    } else {
        format!(
            " ⚠ Baseline health: prompt(s) {degenerate:?} have a near-constant baseline \
             (deterministic or cached outputs), so their signal reflects embedding noise, not \
             behaviour — re-capture a varied baseline before trusting it."
        )
    };

    // Exchangeability caveat: a model-version change since baseline capture
    // means the run is compared against a different model's distribution.
    let version_note = match model_change {
        Some((before, after)) => format!(
            " ⚠ Model version changed since baseline capture ({before} → {after}): the baseline \
             describes a different model, so this verdict may be confounded — re-capture the \
             baseline against the current model."
        ),
        None => String::new(),
    };

    if assessment.level == DriftLevel::None {
        return format!(
            "No drift detected: the run's outputs are statistically consistent with the baseline \
             ({method} test, combined p = {p:.4} ≥ target FPR {target_fpr:.4}).{health_note}\
             {version_note}"
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
         FPR {target_fpr:.4}, magnitude {effect:.1} SD), so the model's outputs have shifted \
         relative to the baseline.{strongest}{health_note}{version_note}",
        effect = assessment.effect_size
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
            effect_size: 3.5,
            level,
            method,
            per_prompt: vec![PromptDrift {
                prompt_index: 2,
                p_value: p,
                n_baseline: 40,
                low_variance_baseline: false,
            }],
        }
    }

    #[test]
    fn none_is_reported_as_consistent() {
        let text = interpret(
            &assessment(DriftLevel::None, 0.4, METHOD_PER_PROMPT),
            0.01,
            None,
        );
        assert!(text.contains("No drift detected"), "{text}");
        assert!(text.contains("consistent"), "{text}");
    }

    #[test]
    fn drift_names_method_and_strongest_prompt() {
        let text = interpret(
            &assessment(DriftLevel::High, 0.0001, METHOD_PER_PROMPT),
            0.01,
            None,
        );
        assert!(text.contains("High drift"), "{text}");
        assert!(text.contains("per-prompt conformal"), "{text}");
        assert!(text.contains("prompt #2"), "{text}");
    }

    #[test]
    fn flags_low_variance_baseline_in_text() {
        let mut a = assessment(DriftLevel::None, 0.4, METHOD_PER_PROMPT);
        a.per_prompt[0].low_variance_baseline = true;
        let text = interpret(&a, 0.01, None);
        assert!(text.contains("Baseline health"), "{text}");
    }

    #[test]
    fn pooled_method_is_named() {
        let mut a = assessment(DriftLevel::Medium, 0.001, METHOD_POOLED);
        a.per_prompt.clear();
        let text = interpret(&a, 0.01, None);
        assert!(text.contains("pooled MMD/energy"), "{text}");
    }

    #[test]
    fn flags_model_version_change_on_both_no_drift_and_drift() {
        // Caveat appears whether or not the run statistically drifted.
        let no_drift = assessment(DriftLevel::None, 0.4, METHOD_PER_PROMPT);
        let text = interpret(
            &no_drift,
            0.01,
            Some(("claude-sonnet-4-6", "claude-opus-4-8")),
        );
        assert!(text.contains("Model version changed"), "{text}");
        assert!(
            text.contains("claude-sonnet-4-6 → claude-opus-4-8"),
            "{text}"
        );

        let drift = assessment(DriftLevel::High, 0.0001, METHOD_PER_PROMPT);
        let text = interpret(&drift, 0.01, Some(("gpt-5.4", "gpt-5.5")));
        assert!(text.contains("Model version changed"), "{text}");
    }
}
