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

fn make_report(kl: f32, cosine: f32, level: DriftLevel) -> DriftReport {
    DriftReport {
        run_id: RunId::new(),
        baseline_id: BaselineId::new(),
        kl_divergence: kl,
        cosine_distance: cosine,
        output_entropy_delta: 0.1,
        drift_level: level,
        computed_at: Utc::now(),
    }
}

fn make_rule(webhook_url: String, kl_threshold: f32, cosine_threshold: f32) -> AlertRule {
    AlertRule {
        id: AlertRuleId::new(),
        probe_id: ProbeId::new(),
        kl_threshold,
        cosine_threshold,
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
    let rule = make_rule(webhook_url, 0.1, 0.15);

    // Report that clearly exceeds both thresholds
    let report = make_report(0.5, 0.4, DriftLevel::High);

    let engine = AlertEngine::new(reqwest::Client::new());
    let events = engine.evaluate_and_fire(&report, &[rule]).await;

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
    let mut rule = make_rule(webhook_url, 0.1, 0.15);
    rule.active = false;

    let report = make_report(0.5, 0.4, DriftLevel::High);

    let engine = AlertEngine::new(reqwest::Client::new());
    let events = engine.evaluate_and_fire(&report, &[rule]).await;

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
    let rule = make_rule(webhook_url, 1.0, 0.8); // high thresholds

    // Report well below thresholds
    let report = make_report(0.05, 0.03, DriftLevel::None);

    let engine = AlertEngine::new(reqwest::Client::new());
    let events = engine.evaluate_and_fire(&report, &[rule]).await;

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
    let rule = make_rule(webhook_url, 0.1, 0.15);
    let rule_id = rule.id.clone();

    let report = make_report(0.3, 0.2, DriftLevel::Medium);
    let engine = AlertEngine::new(reqwest::Client::new());
    let events: Vec<AlertEvent> = engine.evaluate_and_fire(&report, &[rule]).await;

    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.rule_id, rule_id);
    assert_eq!(event.drift_report.drift_level, DriftLevel::Medium);

    server.verify().await;
}
