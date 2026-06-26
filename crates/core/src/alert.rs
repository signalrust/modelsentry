//! Alert engine — evaluates [`DriftReport`]s against [`AlertRule`]s and fires
//! notifications to configured channels.

use std::net::IpAddr;

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
    /// When `false` (the secure default), webhook/Slack URLs that resolve to a
    /// private, loopback, link-local, or otherwise non-public address are
    /// refused before any request is sent — closing the SSRF hole that an
    /// attacker-supplied `http://169.254.169.254/…` or `http://localhost/…`
    /// rule would otherwise open. Set `true` only for trusted internal
    /// receivers.
    allow_private_targets: bool,
}

impl AlertEngine {
    /// Create a new engine backed by the given HTTP client.
    ///
    /// Private/loopback webhook targets are **blocked** by default; use
    /// [`AlertEngine::with_allow_private_targets`] to opt in.
    #[must_use]
    pub fn new(http_client: reqwest::Client) -> Self {
        Self {
            http_client,
            allow_private_targets: false,
        }
    }

    /// Allow (or block) webhook targets that resolve to private/loopback/
    /// link-local addresses. Defaults to `false`.
    #[must_use]
    pub fn with_allow_private_targets(mut self, allow: bool) -> Self {
        self.allow_private_targets = allow;
        self
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
                if let Err(reason) = self.validate_target(url).await {
                    tracing::warn!("refusing to fire webhook to {url}: {reason}");
                    return;
                }
                let body = serde_json::json!({
                    "event_id": event.id,
                    "rule_id": event.rule_id,
                    "drift_level": event.drift_report.drift_level,
                    "fired_at": event.fired_at,
                });
                if let Err(e) = self.http_client.post(url).json(&body).send().await {
                    tracing::warn!("failed to fire webhook to {url}: {e}");
                } else {
                    tracing::info!("webhook fired successfully to {url}");
                }
            }
            AlertChannel::Email { address } => {
                // TODO(Task 5.1): email via SMTP — skipped for now
                tracing::info!("alert email would be sent to {address}");
            }
        }
    }

    /// SSRF guard: validate a webhook/Slack URL before sending to it.
    ///
    /// Rejects non-`http(s)` schemes, then (unless private targets are
    /// allowed) resolves the host and refuses if **any** resolved address is
    /// non-public. Resolving up front blocks the realistic attacks
    /// (cloud-metadata `169.254.169.254`, `localhost`, RFC-1918 ranges); a
    /// determined attacker controlling DNS could still rebind between this
    /// check and the request — pin the target IP if that threat applies.
    async fn validate_target(&self, url: &str) -> std::result::Result<(), String> {
        let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => return Err(format!("unsupported scheme '{other}' (only http/https)")),
        }
        if self.allow_private_targets {
            return Ok(());
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| "URL has no host".to_string())?;
        let port = parsed.port_or_known_default().unwrap_or(443);

        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| format!("failed to resolve host '{host}': {e}"))?
            .collect();
        if addrs.is_empty() {
            return Err(format!("host '{host}' resolved to no addresses"));
        }
        for addr in addrs {
            if is_disallowed_ip(addr.ip()) {
                return Err(format!(
                    "target resolves to a non-public address ({})",
                    addr.ip()
                ));
            }
        }
        Ok(())
    }
}

/// Returns `true` for any address an outbound webhook must not reach:
/// loopback, unspecified, multicast, RFC-1918 private, link-local (incl. the
/// `169.254.169.254` cloud-metadata endpoint), IPv6 unique-local/link-local,
/// and IPv4-mapped IPv6 forms of all the above.
fn is_disallowed_ip(ip: IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_disallowed_ip(IpAddr::V4(mapped));
            }
            let seg0 = v6.segments()[0];
            (seg0 & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (seg0 & 0xfe00) == 0xfc00 // unique-local fc00::/7
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

        // The mock listens on loopback, so the SSRF guard must be relaxed.
        let engine = AlertEngine::default().with_allow_private_targets(true);
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

    // ── SSRF guard ────────────────────────────────────────────────────────────

    #[test]
    fn is_disallowed_ip_classifies_addresses() {
        use std::net::IpAddr;
        let blocked = [
            "127.0.0.1",              // loopback v4
            "0.0.0.0",                // unspecified
            "10.1.2.3",               // private
            "172.16.5.4",             // private
            "192.168.1.1",            // private
            "169.254.169.254",        // link-local (cloud metadata)
            "255.255.255.255",        // broadcast
            "224.0.0.1",              // multicast
            "::1",                    // loopback v6
            "fe80::1",                // link-local v6
            "fc00::1",                // unique-local v6
            "fd12:3456::1",           // unique-local v6
            "::ffff:127.0.0.1",       // IPv4-mapped loopback
            "::ffff:169.254.169.254", // IPv4-mapped metadata
        ];
        for ip in blocked {
            assert!(
                is_disallowed_ip(ip.parse::<IpAddr>().unwrap()),
                "{ip} should be blocked"
            );
        }

        let allowed = ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:2800:220:1::1"];
        for ip in allowed {
            assert!(
                !is_disallowed_ip(ip.parse::<IpAddr>().unwrap()),
                "{ip} should be allowed"
            );
        }
    }

    #[tokio::test]
    async fn validate_target_rejects_non_http_scheme() {
        let engine = AlertEngine::default();
        let err = engine
            .validate_target("file:///etc/passwd")
            .await
            .unwrap_err();
        assert!(err.contains("unsupported scheme"), "{err}");
    }

    #[tokio::test]
    async fn validate_target_blocks_private_and_metadata_literals() {
        // Literal IPs resolve without real DNS, so this is deterministic/offline.
        let engine = AlertEngine::default();
        for url in [
            "http://127.0.0.1/hook",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/hook",
            "http://[::1]/hook",
        ] {
            let err = engine.validate_target(url).await.unwrap_err();
            assert!(err.contains("non-public"), "{url} -> {err}");
        }
    }

    #[tokio::test]
    async fn validate_target_allows_public_literal() {
        let engine = AlertEngine::default();
        // 8.8.8.8 is a literal — only parsed/classified, never connected to.
        assert!(engine.validate_target("https://8.8.8.8/hook").await.is_ok());
    }

    #[tokio::test]
    async fn validate_target_allows_anything_when_private_allowed() {
        let engine = AlertEngine::default().with_allow_private_targets(true);
        assert!(
            engine
                .validate_target("http://127.0.0.1/hook")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn secure_default_blocks_loopback_webhook_end_to_end() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let server = MockServer::start().await;
        // The guard must prevent the request entirely → zero calls.
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let engine = AlertEngine::default(); // secure default → blocks loopback
        let report = make_report(5.0, 0.0);
        let rule = AlertRule {
            id: AlertRuleId::new(),
            probe_id: ProbeId::new(),
            kl_threshold: 1.0,
            cosine_threshold: 99.0,
            channels: vec![AlertChannel::Webhook {
                url: format!("{}/hook", server.uri()),
            }],
            active: true,
        };
        // Event is still produced (drift happened); only delivery is suppressed.
        let events = engine.evaluate_and_fire(&report, &[rule]).await;
        assert_eq!(events.len(), 1);
        // wiremock asserts .expect(0) on drop.
    }
}
