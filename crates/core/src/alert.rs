//! Alert engine — evaluates [`DriftReport`]s against [`AlertRule`]s and fires
//! notifications to configured channels.

use chrono::Utc;
use uuid::Uuid;

use modelsentry_common::models::{AlertChannel, AlertEvent, AlertRule, DriftReport};

/// Evaluates drift reports against alert rules and fires notifications over
/// HTTP webhooks, Slack, or (in a future task) email.
///
/// Cheaply cloneable via the inner [`reqwest::Client`].
#[derive(Clone)]
pub struct AlertEngine {
    http_client: reqwest::Client,
}

impl AlertEngine {
    /// Create a new engine backed by the given HTTP client.
    #[must_use]
    pub fn new(http_client: reqwest::Client) -> Self {
        Self { http_client }
    }

    /// Evaluate `report` against every active rule. For each triggered rule,
    /// fire all configured channels and collect the resulting [`AlertEvent`]s.
    ///
    /// Channel delivery errors are logged at `WARN` level but do not abort
    /// processing of other rules or channels.
    pub async fn evaluate_and_fire(
        &self,
        report: &DriftReport,
        rules: &[AlertRule],
    ) -> Vec<AlertEvent> {
        let mut events = Vec::new();
        for rule in rules {
            if !rule.active {
                continue;
            }
            if Self::is_triggered(report, rule) {
                let event = AlertEvent {
                    id: Uuid::new_v4(),
                    rule_id: rule.id.clone(),
                    drift_report: report.clone(),
                    fired_at: Utc::now(),
                    acknowledged: false,
                };
                for channel in &rule.channels {
                    self.fire_channel(channel, &event).await;
                }
                events.push(event);
            }
        }
        events
    }

    /// Returns `true` if the report breaches any threshold defined by `rule`.
    fn is_triggered(report: &DriftReport, rule: &AlertRule) -> bool {
        report.kl_divergence > rule.kl_threshold || report.cosine_distance > rule.cosine_threshold
    }

    /// Send a notification for `event` to `channel`, logging on failure.
    async fn fire_channel(&self, channel: &AlertChannel, event: &AlertEvent) {
        match channel {
            AlertChannel::Webhook { url } | AlertChannel::Slack { webhook_url: url } => {
                let body = serde_json::json!({
                    "event_id": event.id,
                    "rule_id": event.rule_id,
                    "drift_level": event.drift_report.drift_level,
                    "fired_at": event.fired_at,
                });
                if let Err(e) = self.http_client.post(url).json(&body).send().await {
                    tracing::warn!("failed to fire webhook to {url}: {e}");
                }
            }
            AlertChannel::Email { address } => {
                // TODO(Task 5.1): email via SMTP — skipped for now
                tracing::info!("alert email would be sent to {address}");
            }
        }
    }
}

impl Default for AlertEngine {
    fn default() -> Self {
        Self::new(reqwest::Client::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelsentry_common::{
        models::{AlertChannel, AlertRule, DriftLevel, DriftReport},
        types::{AlertRuleId, BaselineId, ProbeId, RunId},
    };

    fn make_report(kl: f32, cos: f32) -> DriftReport {
        DriftReport {
            run_id: RunId::new(),
            baseline_id: BaselineId::new(),
            kl_divergence: kl,
            cosine_distance: cos,
            output_entropy_delta: 0.0,
            drift_level: DriftLevel::None,
            computed_at: Utc::now(),
        }
    }

    fn make_rule(kl_threshold: f32, cosine_threshold: f32, active: bool) -> AlertRule {
        AlertRule {
            id: AlertRuleId::new(),
            probe_id: ProbeId::new(),
            kl_threshold,
            cosine_threshold,
            channels: vec![],
            active,
        }
    }

    #[tokio::test]
    async fn no_rules_returns_empty_events() {
        let engine = AlertEngine::default();
        let report = make_report(1.0, 0.9);
        let events = engine.evaluate_and_fire(&report, &[]).await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn inactive_rule_does_not_fire() {
        let engine = AlertEngine::default();
        let report = make_report(100.0, 100.0);
        let rule = make_rule(0.01, 0.01, false);
        let events = engine.evaluate_and_fire(&report, &[rule]).await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn rule_triggers_when_kl_exceeds_threshold() {
        let engine = AlertEngine::default();
        let report = make_report(1.5, 0.0);
        let rule = make_rule(1.0, 99.0, true);
        let events = engine.evaluate_and_fire(&report, &[rule]).await;
        assert_eq!(events.len(), 1);
        assert!(!events[0].acknowledged);
    }

    #[tokio::test]
    async fn rule_triggers_when_cosine_exceeds_threshold() {
        let engine = AlertEngine::default();
        let report = make_report(0.0, 0.9);
        let rule = make_rule(99.0, 0.5, true);
        let events = engine.evaluate_and_fire(&report, &[rule]).await;
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn rule_does_not_fire_when_below_both_thresholds() {
        let engine = AlertEngine::default();
        let report = make_report(0.1, 0.1);
        let rule = make_rule(1.0, 1.0, true);
        let events = engine.evaluate_and_fire(&report, &[rule]).await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn event_references_correct_rule() {
        let engine = AlertEngine::default();
        let report = make_report(10.0, 0.0);
        let rule = make_rule(1.0, 99.0, true);
        let rule_id = rule.id.clone();
        let events = engine.evaluate_and_fire(&report, &[rule]).await;
        assert_eq!(events[0].rule_id, rule_id);
    }

    #[tokio::test]
    async fn webhook_channel_fires_against_mock_server() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let engine = AlertEngine::default();
        let report = make_report(5.0, 0.0);
        let url = format!("{}/hook", server.uri());
        let rule = AlertRule {
            id: AlertRuleId::new(),
            probe_id: ProbeId::new(),
            kl_threshold: 1.0,
            cosine_threshold: 99.0,
            channels: vec![AlertChannel::Webhook { url }],
            active: true,
        };
        engine.evaluate_and_fire(&report, &[rule]).await;
        // wiremock verifies the expectation on drop
    }
}
