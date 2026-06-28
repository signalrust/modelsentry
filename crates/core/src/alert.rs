//! Alert engine — evaluates [`DriftReport`]s against [`AlertRule`]s and fires
//! notifications to configured channels.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use modelsentry_common::models::{AlertChannel, AlertEvent, AlertRule, DriftLevel, DriftReport};
use modelsentry_common::types::AlertRuleId;

use crate::email::EmailMailer;

/// Evaluates drift reports against alert rules and fires notifications over
/// HTTP webhooks, Slack, or email (SMTP).
///
/// Cheaply cloneable via the inner [`reqwest::Client`] and shared mailer.
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
    /// Minimum gap between notifications for the same rule. A triggered rule
    /// whose previous fire was less than this ago is **de-duplicated** (no new
    /// event/notification), bounding repeat alerts on persistently-drifted or
    /// chronically-noisy probes. [`Duration::zero`] (the default) disables it.
    cooldown: Duration,
    /// SMTP mailer for the email channel, built at startup from `[alerts.smtp]`.
    /// `None` ⇒ email is unconfigured, so [`AlertChannel::Email`] is logged and
    /// skipped rather than silently dropped.
    mailer: Option<Arc<EmailMailer>>,
    /// Rolling-window sequential control (alpha-spending). `None` (the default)
    /// tests every run independently at the rule's `target_fpr`. When set, each
    /// rule may spend at most [`SequentialControl::alpha_budget`] of testing
    /// level over a [`SequentialControl::window`], bounding the expected number
    /// of false alarms per rule per window.
    sequential: Option<SequentialControl>,
}

/// Rolling-window alpha-spending parameters for sequential control.
///
/// Caps the **expected number of false alarms** per rule per `window` at
/// `alpha_budget`. The engine spends each look's testing level from the budget
/// (debit-on-look), so the cumulative per-window spend telescopes to exactly
/// `alpha_budget` and the rule falls silent once it is exhausted — until older
/// spends age out of the window.
#[derive(Debug, Clone, Copy)]
pub struct SequentialControl {
    /// Length of the rolling window over which the budget applies.
    pub window: Duration,
    /// Expected false alarms tolerated per rule per `window`.
    pub alpha_budget: f64,
}

/// One rule's alpha debit for a single run (debit-on-look). The caller persists
/// these to the spend ledger so the budget survives restarts and spans runs.
#[derive(Debug, Clone)]
pub struct RuleSpend {
    /// Rule that was looked at.
    pub rule_id: AlertRuleId,
    /// Testing level spent on this look (`min(target_fpr, remaining_budget)`).
    pub alpha: f64,
}

/// Result of [`AlertEngine::evaluate_and_fire`]: the events that fired plus the
/// alpha debited from each rule's window budget this run.
#[derive(Debug, Clone, Default)]
pub struct AlertOutcome {
    /// Events for rules that triggered and were not suppressed.
    pub events: Vec<AlertEvent>,
    /// Per-rule alpha spends to persist. Empty when sequential control is off.
    pub spends: Vec<RuleSpend>,
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
            cooldown: Duration::zero(),
            mailer: None,
            sequential: None,
        }
    }

    /// Allow (or block) webhook targets that resolve to private/loopback/
    /// link-local addresses. Defaults to `false`.
    #[must_use]
    pub fn with_allow_private_targets(mut self, allow: bool) -> Self {
        self.allow_private_targets = allow;
        self
    }

    /// Set the per-rule alert cooldown / de-duplication window. A non-positive
    /// duration disables it (every triggered run alerts). Defaults to disabled.
    #[must_use]
    pub fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown = cooldown;
        self
    }

    /// Attach the SMTP mailer used by the email channel. `None` (the default)
    /// leaves email unconfigured: email alerts are logged and skipped.
    #[must_use]
    pub fn with_mailer(mut self, mailer: Option<Arc<EmailMailer>>) -> Self {
        self.mailer = mailer;
        self
    }

    /// Enable rolling-window sequential control (alpha-spending). `None` (the
    /// default) tests every run independently at `target_fpr`. A control whose
    /// `alpha_budget` is non-positive is treated as disabled.
    #[must_use]
    pub fn with_sequential(mut self, sequential: Option<SequentialControl>) -> Self {
        self.sequential = sequential.filter(|s| s.alpha_budget > 0.0);
        self
    }

    /// The rolling window over which alpha is budgeted, or `None` when
    /// sequential control is disabled. The caller uses it to bound the spend
    /// ledger lookup (`now − window`) and to prune aged-out spends.
    #[must_use]
    pub fn sequential_window(&self) -> Option<Duration> {
        self.sequential.map(|s| s.window)
    }

    /// Evaluate `report` against every active rule. For each triggered rule that
    /// is **not** within its cooldown window or out of alpha budget, fire all
    /// configured channels and collect the resulting [`AlertEvent`]s.
    ///
    /// `last_fired` maps a rule id to the timestamp it most recently fired (the
    /// caller loads this from the store); a rule absent from the map has never
    /// fired. A triggered rule whose last fire is newer than [`Self::cooldown`]
    /// is de-duplicated — the run is still assessed, only the notification is
    /// suppressed.
    ///
    /// `spent_alpha` maps a rule id to the alpha it has already spent inside the
    /// current window (the caller sums the spend ledger over [the window]). It
    /// is consulted only when sequential control is enabled: the rule is tested
    /// at `min(target_fpr, alpha_budget − spent)`, and the returned
    /// [`AlertOutcome::spends`] records what each look debited so the caller can
    /// persist it. **Every active rule looked at spends** (debit-on-look),
    /// whether or not it fired, so a budget bounds the expected false alarms
    /// regardless of cooldown suppression.
    ///
    /// Channel delivery errors are logged at `WARN` level but do not abort
    /// processing of other rules or channels.
    ///
    /// [the window]: SequentialControl::window
    pub async fn evaluate_and_fire(
        &self,
        report: &DriftReport,
        rules: &[AlertRule],
        last_fired: &HashMap<AlertRuleId, DateTime<Utc>>,
        spent_alpha: &HashMap<AlertRuleId, f64>,
    ) -> AlertOutcome {
        let mut outcome = AlertOutcome::default();
        let now = Utc::now();
        for rule in rules {
            if !rule.active {
                continue;
            }
            // Effective testing level for this look: the rule's `target_fpr`,
            // capped by the budget remaining in the window when sequential
            // control is on. A look at level `alpha` spends `alpha` from the
            // budget (debit-on-look); the per-window sum telescopes to exactly
            // `alpha_budget`, so the expected false alarms are bounded.
            let alpha = self.effective_alpha(rule, spent_alpha);
            if self.sequential.is_some() && alpha > 0.0 {
                outcome.spends.push(RuleSpend {
                    rule_id: rule.id.clone(),
                    alpha,
                });
            }
            if f64::from(report.combined_p_value) >= alpha {
                // Not significant at this look's level. When the budget is
                // exhausted `alpha` is 0, so the rule can never fire.
                continue;
            }
            if self.in_cooldown(rule, last_fired, now) {
                tracing::debug!(
                    rule_id = %rule.id,
                    "alert de-duplicated: rule fired within its cooldown window",
                );
                continue;
            }
            let event = AlertEvent {
                id: Uuid::new_v4(),
                rule_id: rule.id.clone(),
                drift_report: report.clone(),
                fired_at: now,
                acknowledged: false,
            };
            for channel in &rule.channels {
                self.fire_channel(channel, &event).await;
            }
            outcome.events.push(event);
        }
        outcome
    }

    /// Testing level for one look at `rule`. Without sequential control this is
    /// the rule's `target_fpr`. With it, the level is capped by the budget left
    /// in the window — `min(target_fpr, alpha_budget − spent)`, never negative —
    /// so a rule with no remaining budget is tested at level 0 (silenced).
    fn effective_alpha(&self, rule: &AlertRule, spent_alpha: &HashMap<AlertRuleId, f64>) -> f64 {
        let target = f64::from(rule.target_fpr);
        match self.sequential {
            None => target,
            Some(seq) => {
                let spent = spent_alpha.get(&rule.id).copied().unwrap_or(0.0);
                let remaining = (seq.alpha_budget - spent).max(0.0);
                target.min(remaining)
            }
        }
    }

    /// Whether `rule` is inside its cooldown window at `now` (its last fire was
    /// less than [`Self::cooldown`] ago). Always `false` when the cooldown is
    /// disabled or the rule has never fired.
    fn in_cooldown(
        &self,
        rule: &AlertRule,
        last_fired: &HashMap<AlertRuleId, DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> bool {
        if self.cooldown <= Duration::zero() {
            return false;
        }
        last_fired
            .get(&rule.id)
            .is_some_and(|&last| now - last < self.cooldown)
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
            AlertChannel::Email { address } => self.send_email(address, event).await,
        }
    }

    /// Deliver `event` to `address` over SMTP. If no mailer is configured
    /// (`[alerts.smtp]` absent), the alert is logged and skipped rather than
    /// silently dropped. Delivery failures are logged at `WARN`.
    async fn send_email(&self, address: &str, event: &AlertEvent) {
        let Some(mailer) = self.mailer.as_ref() else {
            tracing::warn!("email alert to {address} skipped: no SMTP configured ([alerts.smtp])");
            return;
        };
        let report = &event.drift_report;
        let subject = format!(
            "[ModelSentry] {} drift alert",
            drift_level_label(&report.drift_level)
        );
        let body = format!(
            "ModelSentry detected drift.\n\n\
             Level:        {level}\n\
             Combined p:   {p:.6} (target FPR {fpr:.6})\n\
             Magnitude:    {effect:.1} SD\n\
             Method:       {method}\n\
             Fired at:     {fired_at}\n\
             Rule:         {rule}\n\
             Run:          {run}\n\n\
             {interpretation}\n",
            level = drift_level_label(&report.drift_level),
            p = report.combined_p_value,
            fpr = report.target_fpr,
            effect = report.effect_size,
            method = report.method,
            fired_at = event.fired_at,
            rule = event.rule_id,
            run = report.run_id,
            interpretation = report.interpretation,
        );
        if let Err(e) = mailer.send(address, &subject, &body).await {
            tracing::warn!("failed to send alert email to {address}: {e}");
        } else {
            tracing::info!("alert email sent to {address}");
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

/// Human-readable label for a [`DriftLevel`], used in alert subjects/bodies.
fn drift_level_label(level: &DriftLevel) -> &'static str {
    match level {
        DriftLevel::None => "No",
        DriftLevel::Low => "Low",
        DriftLevel::Medium => "Medium",
        DriftLevel::High => "High",
        DriftLevel::Critical => "Critical",
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

    fn make_report(combined_p_value: f32) -> DriftReport {
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
            drift_level: DriftLevel::None,
            interpretation: String::new(),
            computed_at: Utc::now(),
        }
    }

    /// Empty fire-history: every rule is treated as never having fired.
    fn no_history() -> HashMap<AlertRuleId, chrono::DateTime<Utc>> {
        HashMap::new()
    }

    /// Empty spend ledger: no rule has spent any alpha this window.
    fn no_spend() -> HashMap<AlertRuleId, f64> {
        HashMap::new()
    }

    fn make_rule(target_fpr: f32, active: bool) -> AlertRule {
        AlertRule {
            id: AlertRuleId::new(),
            probe_id: ProbeId::new(),
            target_fpr,
            channels: vec![],
            active,
        }
    }

    #[tokio::test]
    async fn no_rules_returns_empty_events() {
        let engine = AlertEngine::default();
        let report = make_report(0.001);
        let events = engine
            .evaluate_and_fire(&report, &[], &no_history(), &no_spend())
            .await
            .events;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn inactive_rule_does_not_fire() {
        let engine = AlertEngine::default();
        let report = make_report(0.0001); // very significant
        let rule = make_rule(0.01, false);
        let events = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await
            .events;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn rule_triggers_when_p_below_target_fpr() {
        let engine = AlertEngine::default();
        let report = make_report(0.001); // p < target_fpr
        let rule = make_rule(0.01, true);
        let events = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await
            .events;
        assert_eq!(events.len(), 1);
        assert!(!events[0].acknowledged);
    }

    #[tokio::test]
    async fn rule_does_not_fire_when_p_above_target_fpr() {
        let engine = AlertEngine::default();
        let report = make_report(0.4); // not significant
        let rule = make_rule(0.01, true);
        let events = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await
            .events;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn event_references_correct_rule() {
        let engine = AlertEngine::default();
        let report = make_report(0.0005);
        let rule = make_rule(0.01, true);
        let rule_id = rule.id.clone();
        let events = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await
            .events;
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
        let report = make_report(0.0001);
        let url = format!("{}/hook", server.uri());
        let rule = AlertRule {
            id: AlertRuleId::new(),
            probe_id: ProbeId::new(),
            target_fpr: 0.01,
            channels: vec![AlertChannel::Webhook { url }],
            active: true,
        };
        engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await;
        // wiremock verifies the expectation on drop
    }

    // ── Cooldown / de-duplication ─────────────────────────────────────────────

    #[tokio::test]
    async fn cooldown_suppresses_a_recently_fired_rule() {
        let engine = AlertEngine::default().with_cooldown(Duration::hours(1));
        let report = make_report(0.0001); // significant → would fire
        let rule = make_rule(0.01, true);
        // The rule fired one minute ago — inside the 1-hour window.
        let mut last_fired = HashMap::new();
        last_fired.insert(rule.id.clone(), Utc::now() - Duration::minutes(1));
        let events = engine
            .evaluate_and_fire(&report, &[rule], &last_fired, &no_spend())
            .await
            .events;
        assert!(
            events.is_empty(),
            "alert should be de-duplicated within cooldown"
        );
    }

    #[tokio::test]
    async fn cooldown_allows_a_rule_whose_window_has_elapsed() {
        let engine = AlertEngine::default().with_cooldown(Duration::hours(1));
        let report = make_report(0.0001);
        let rule = make_rule(0.01, true);
        // Last fire was two hours ago — outside the 1-hour window.
        let mut last_fired = HashMap::new();
        last_fired.insert(rule.id.clone(), Utc::now() - Duration::hours(2));
        let events = engine
            .evaluate_and_fire(&report, &[rule], &last_fired, &no_spend())
            .await
            .events;
        assert_eq!(
            events.len(),
            1,
            "alert should fire once the cooldown elapsed"
        );
    }

    #[tokio::test]
    async fn zero_cooldown_never_suppresses() {
        let engine = AlertEngine::default(); // cooldown disabled (zero)
        let report = make_report(0.0001);
        let rule = make_rule(0.01, true);
        let mut last_fired = HashMap::new();
        last_fired.insert(rule.id.clone(), Utc::now()); // fired "just now"
        let events = engine
            .evaluate_and_fire(&report, &[rule], &last_fired, &no_spend())
            .await
            .events;
        assert_eq!(events.len(), 1, "disabled cooldown must not suppress");
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
        let report = make_report(0.0001);
        let rule = AlertRule {
            id: AlertRuleId::new(),
            probe_id: ProbeId::new(),
            target_fpr: 0.01,
            channels: vec![AlertChannel::Webhook {
                url: format!("{}/hook", server.uri()),
            }],
            active: true,
        };
        // Event is still produced (drift happened); only delivery is suppressed.
        let events = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await
            .events;
        assert_eq!(events.len(), 1);
        // wiremock asserts .expect(0) on drop.
    }

    // ── Sequential control / alpha-spending ───────────────────────────────────

    /// A control over the configured default window with the given per-rule
    /// budget. The window length is irrelevant to the engine (only the store's
    /// pruning consults it), so it is sourced from the SSOT default rather than
    /// a magic literal.
    fn seq(alpha_budget: f64) -> SequentialControl {
        let window_secs =
            i64::try_from(modelsentry_common::constants::alerts::SEQUENTIAL_WINDOW_SECS)
                .unwrap_or(i64::MAX);
        SequentialControl {
            window: Duration::seconds(window_secs),
            alpha_budget,
        }
    }

    #[tokio::test]
    async fn sequential_disabled_records_no_spend() {
        let engine = AlertEngine::default(); // no sequential control
        let report = make_report(0.0001);
        let rule = make_rule(0.01, true);
        let outcome = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await;
        assert_eq!(outcome.events.len(), 1);
        assert!(outcome.spends.is_empty(), "disabled control must not spend");
    }

    #[tokio::test]
    async fn sequential_fires_and_debits_within_budget() {
        let engine = AlertEngine::default().with_sequential(Some(seq(0.05)));
        let rule = make_rule(0.01, true);
        let report = make_report(0.0001); // clears target_fpr
        let outcome = engine
            .evaluate_and_fire(
                &report,
                std::slice::from_ref(&rule),
                &no_history(),
                &no_spend(),
            )
            .await;
        assert_eq!(outcome.events.len(), 1);
        assert_eq!(outcome.spends.len(), 1);
        assert_eq!(outcome.spends[0].rule_id, rule.id);
        // A full-sensitivity look spends exactly target_fpr.
        assert!((outcome.spends[0].alpha - 0.01).abs() < 1e-9);
    }

    #[tokio::test]
    async fn sequential_debits_every_look_even_when_not_triggered() {
        let engine = AlertEngine::default().with_sequential(Some(seq(0.05)));
        let report = make_report(0.4); // p above target_fpr → does not fire
        let rule = make_rule(0.01, true);
        let outcome = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await;
        assert!(outcome.events.is_empty(), "p above level must not fire");
        // Looking at the data spends alpha whether or not it fired.
        assert_eq!(outcome.spends.len(), 1);
        assert!((outcome.spends[0].alpha - 0.01).abs() < 1e-9);
    }

    #[tokio::test]
    async fn sequential_exhausted_budget_silences_rule() {
        let engine = AlertEngine::default().with_sequential(Some(seq(0.05)));
        let report = make_report(0.0001); // extremely significant
        let rule = make_rule(0.01, true);
        // The window's budget is already fully spent.
        let mut spent = HashMap::new();
        spent.insert(rule.id.clone(), 0.05);
        let outcome = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &spent)
            .await;
        assert!(outcome.events.is_empty(), "no budget ⇒ cannot fire");
        assert!(
            outcome.spends.is_empty(),
            "an exhausted budget spends nothing further"
        );
    }

    #[tokio::test]
    async fn sequential_caps_alpha_to_remaining_budget() {
        let engine = AlertEngine::default().with_sequential(Some(seq(0.05)));
        let rule = make_rule(0.01, true);
        // Only 0.004 of budget remains — below the nominal target_fpr of 0.01.
        let mut spent = HashMap::new();
        spent.insert(rule.id.clone(), 0.046);
        // p = 0.005 clears target_fpr (0.01) but NOT the budget-capped 0.004.
        let report = make_report(0.005);
        let outcome = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &spent)
            .await;
        assert!(
            outcome.events.is_empty(),
            "p must clear the budget-capped level, not the nominal target_fpr"
        );
        assert!((outcome.spends[0].alpha - 0.004).abs() < 1e-9);
    }

    #[tokio::test]
    async fn sequential_with_zero_budget_is_disabled() {
        // A control with a non-positive budget is treated as off by the builder.
        let engine = AlertEngine::default().with_sequential(Some(seq(0.0)));
        let report = make_report(0.0001);
        let rule = make_rule(0.01, true);
        let outcome = engine
            .evaluate_and_fire(&report, &[rule], &no_history(), &no_spend())
            .await;
        assert_eq!(outcome.events.len(), 1, "zero budget ⇒ control disabled");
        assert!(outcome.spends.is_empty());
    }
}
