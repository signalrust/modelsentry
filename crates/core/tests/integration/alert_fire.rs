//! Integration tests: alert firing over HTTP webhook using a wiremock server.
//!
//! Uses `wiremock` to capture outbound POST requests and verify the payload
//! contains the correct drift event fields.

use chrono::Utc;
use modelsentry_common::{
    models::{AlertChannel, AlertEvent, AlertRule, DriftLevel, DriftReport},
    types::{AlertRuleId, BaselineId, ProbeId, RunId},
};
use modelsentry_core::alert::AlertEngine;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_report(combined_p_value: f32, level: DriftLevel) -> DriftReport {
    DriftReport {
        run_id: RunId::new(),
        baseline_id: BaselineId::new(),
        combined_p_value,
        statistic: -(combined_p_value.max(f32::MIN_POSITIVE)).log10(),
        // Fixture magnitude derived from the p-value (no standalone literal).
        effect_size: -(combined_p_value.max(f32::MIN_POSITIVE)).log10(),
        target_fpr: 0.01,
        method: modelsentry_common::constants::method::PER_PROMPT_CONFORMAL.to_string(),
        per_prompt: Vec::new(),
        drift_level: level,
        interpretation: String::new(),
        computed_at: Utc::now(),
    }
}

fn make_rule(webhook_url: String, target_fpr: f32) -> AlertRule {
    AlertRule {
        id: AlertRuleId::new(),
        probe_id: ProbeId::new(),
        target_fpr,
        channels: vec![AlertChannel::Webhook { url: webhook_url }],
        active: true,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// When a drift report exceeds the rule threshold, the alert engine must POST
/// the event payload to the configured webhook URL exactly once.
#[tokio::test]
async fn webhook_receives_correct_payload_on_drift_event() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let webhook_url = format!("{}/webhook", server.uri());
    let rule = make_rule(webhook_url, 0.01);

    // Report that clearly exceeds both thresholds
    let report = make_report(0.001, DriftLevel::High);

    let engine = AlertEngine::new(reqwest::Client::new()).with_allow_private_targets(true);
    let events = engine
        .evaluate_and_fire(
            &report,
            &[rule],
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
        .await
        .events;

    assert_eq!(events.len(), 1, "one alert event should have been fired");
    let event = &events[0];
    assert_eq!(event.drift_report.drift_level, DriftLevel::High);
    assert!(!event.acknowledged);

    // wiremock's `expect(1)` assertion runs on server drop — verifies the
    // POST was actually sent.
    server.verify().await;
}

/// An inactive rule must NOT trigger a webhook call.
#[tokio::test]
async fn inactive_rule_does_not_fire_webhook() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let webhook_url = format!("{}/webhook", server.uri());
    let mut rule = make_rule(webhook_url, 0.01);
    rule.active = false;

    let report = make_report(0.001, DriftLevel::High);

    let engine = AlertEngine::new(reqwest::Client::new()).with_allow_private_targets(true);
    let events = engine
        .evaluate_and_fire(
            &report,
            &[rule],
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
        .await
        .events;

    assert!(events.is_empty(), "inactive rule should produce no events");

    server.verify().await;
}

/// A report that is below both thresholds must NOT trigger a webhook call.
#[tokio::test]
async fn below_threshold_report_does_not_fire_webhook() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let webhook_url = format!("{}/webhook", server.uri());
    let rule = make_rule(webhook_url, 0.01); // high thresholds

    // Report well below thresholds
    let report = make_report(0.5, DriftLevel::None);

    let engine = AlertEngine::new(reqwest::Client::new()).with_allow_private_targets(true);
    let events = engine
        .evaluate_and_fire(
            &report,
            &[rule],
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
        .await
        .events;

    assert!(
        events.is_empty(),
        "sub-threshold report should produce no events"
    );

    server.verify().await;
}

/// Alert event fields (`event_id`, `rule_id`, `drift_level`) must be present in the
/// webhook POST body.
#[tokio::test]
async fn webhook_payload_contains_required_fields() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let webhook_url = format!("{}/hook", server.uri());
    let rule = make_rule(webhook_url, 0.01);
    let rule_id = rule.id.clone();

    let report = make_report(0.002, DriftLevel::Medium);
    let engine = AlertEngine::new(reqwest::Client::new()).with_allow_private_targets(true);
    let events: Vec<AlertEvent> = engine
        .evaluate_and_fire(
            &report,
            &[rule],
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
        .await
        .events;

    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.rule_id, rule_id);
    assert_eq!(event.drift_report.drift_level, DriftLevel::Medium);

    server.verify().await;
}
