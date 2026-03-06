use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{AlertRuleId, BaselineId, ProbeId, RunId};

/// A configured probe — a named set of prompts sent to one provider/model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub id: ProbeId,
    pub name: String,
    pub provider: ProviderKind,
    pub model: String,
    pub prompts: Vec<ProbePrompt>,
    pub schedule: ProbeSchedule,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbePrompt {
    pub id: Uuid,
    pub text: String,
    pub expected_contains: Option<String>,
    pub expected_not_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Ollama {
        base_url: String,
    },
    AzureOpenAi {
        endpoint: String,
        deployment: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProbeSchedule {
    Cron { expression: String },
    EveryMinutes { minutes: u32 },
}

/// A frozen statistical snapshot — the reference point for drift detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineSnapshot {
    pub id: BaselineId,
    pub probe_id: ProbeId,
    pub captured_at: DateTime<Utc>,
    pub embedding_centroid: Vec<f32>,
    pub embedding_variance: f32,
    pub output_tokens: Vec<Vec<String>>,
    pub run_id: RunId,
}

/// Results of a single probe run (one full pass of all prompts)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRun {
    pub id: RunId,
    pub probe_id: ProbeId,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub embeddings: Vec<Vec<f32>>,
    pub completions: Vec<String>,
    pub drift_report: Option<DriftReport>,
    pub status: RunStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    PartialFailure,
    Failed,
}

/// Statistical comparison between a run and its baseline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub run_id: RunId,
    pub baseline_id: BaselineId,
    pub kl_divergence: f32,
    pub cosine_distance: f32,
    pub output_entropy_delta: f32,
    pub drift_level: DriftLevel,
    pub computed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: AlertRuleId,
    pub probe_id: ProbeId,
    pub kl_threshold: f32,
    pub cosine_threshold: f32,
    pub channels: Vec<AlertChannel>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AlertChannel {
    Webhook { url: String },
    Slack { webhook_url: String },
    Email { address: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: Uuid,
    pub rule_id: AlertRuleId,
    pub drift_report: DriftReport,
    pub fired_at: DateTime<Utc>,
    pub acknowledged: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_serializes_and_deserializes_round_trip() {
        let probe = Probe {
            id: ProbeId::new(),
            name: "test-probe".to_string(),
            provider: ProviderKind::OpenAi,
            model: "gpt-4o".to_string(),
            prompts: vec![ProbePrompt {
                id: Uuid::new_v4(),
                text: "Hello?".to_string(),
                expected_contains: None,
                expected_not_contains: None,
            }],
            schedule: ProbeSchedule::EveryMinutes { minutes: 60 },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&probe).unwrap();
        let probe2: Probe = serde_json::from_str(&json).unwrap();
        assert_eq!(probe.id, probe2.id);
        assert_eq!(probe.name, probe2.name);
    }

    #[test]
    fn drift_level_ordering_is_correct() {
        let levels = [
            DriftLevel::None,
            DriftLevel::Low,
            DriftLevel::Medium,
            DriftLevel::High,
            DriftLevel::Critical,
        ];
        let serialized: Vec<String> = levels
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect();
        assert_eq!(serialized[0], "\"none\"");
        assert_eq!(serialized[4], "\"critical\"");
        let unique: std::collections::HashSet<_> = serialized.iter().collect();
        assert_eq!(unique.len(), 5);
    }

    #[test]
    fn provider_kind_tagged_json_shape() {
        let p = ProviderKind::Ollama {
            base_url: "http://localhost:11434".to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"kind\":\"ollama\""));
        assert!(json.contains("base_url"));
    }

    #[test]
    fn baseline_snapshot_preserves_embedding_dims_after_roundtrip() {
        let snap = BaselineSnapshot {
            id: BaselineId::new(),
            probe_id: ProbeId::new(),
            captured_at: Utc::now(),
            embedding_centroid: vec![0.1, 0.2, 0.3],
            embedding_variance: 0.05,
            output_tokens: vec![vec!["hello".to_string(), "world".to_string()]],
            run_id: RunId::new(),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let snap2: BaselineSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap.embedding_centroid, snap2.embedding_centroid);
        assert_eq!(snap.output_tokens, snap2.output_tokens);
    }

    #[test]
    fn drift_level_ord_none_less_than_critical() {
        assert!(DriftLevel::None < DriftLevel::Low);
        assert!(DriftLevel::Low < DriftLevel::Medium);
        assert!(DriftLevel::Medium < DriftLevel::High);
        assert!(DriftLevel::High < DriftLevel::Critical);
    }

    #[test]
    fn probe_run_roundtrip_json() {
        let run = ProbeRun {
            id: RunId::new(),
            probe_id: ProbeId::new(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            embeddings: vec![vec![0.1, 0.2]],
            completions: vec!["hello world".to_string()],
            drift_report: None,
            status: RunStatus::Success,
        };
        let json = serde_json::to_string(&run).unwrap();
        let run2: ProbeRun = serde_json::from_str(&json).unwrap();
        assert_eq!(run.id, run2.id);
        assert_eq!(run.completions, run2.completions);
    }

    #[test]
    fn alert_rule_roundtrip_json() {
        let rule = AlertRule {
            id: AlertRuleId::new(),
            probe_id: ProbeId::new(),
            kl_threshold: 0.05,
            cosine_threshold: 0.1,
            channels: vec![AlertChannel::Webhook {
                url: "https://example.com/hook".to_string(),
            }],
            active: true,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let rule2: AlertRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule.id, rule2.id);
        assert!(rule2.active);
    }

    #[test]
    fn alert_event_roundtrip_json() {
        let event = AlertEvent {
            id: Uuid::new_v4(),
            rule_id: AlertRuleId::new(),
            drift_report: DriftReport {
                run_id: RunId::new(),
                baseline_id: BaselineId::new(),
                kl_divergence: 0.3,
                cosine_distance: 0.2,
                output_entropy_delta: 0.1,
                drift_level: DriftLevel::High,
                computed_at: Utc::now(),
            },
            fired_at: Utc::now(),
            acknowledged: false,
        };
        let json = serde_json::to_string(&event).unwrap();
        let event2: AlertEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event.id, event2.id);
        assert!(!event2.acknowledged);
    }

    #[test]
    fn probe_schedule_cron_tagged_json_shape() {
        let s = ProbeSchedule::Cron {
            expression: "0 */5 * * *".to_string(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"kind\":\"cron\""));
        assert!(json.contains("expression"));
    }

    #[test]
    fn alert_channel_variants_tagged_json() {
        let slack = AlertChannel::Slack {
            webhook_url: "https://hooks.slack.com/test".to_string(),
        };
        let json = serde_json::to_string(&slack).unwrap();
        assert!(json.contains("\"kind\":\"slack\""));

        let email = AlertChannel::Email {
            address: "test@example.com".to_string(),
        };
        let json = serde_json::to_string(&email).unwrap();
        assert!(json.contains("\"kind\":\"email\""));
    }
}
