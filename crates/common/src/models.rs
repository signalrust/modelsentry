use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::constants::provider;
use crate::types::{AlertRuleId, BaselineId, ProbeId, RunId};

/// A configured probe — a named set of prompts sent to one provider/model.
///
/// The full provider target (kind + model/deployment + any instance params)
/// lives in [`ProviderSpec`]; the runtime constructs exactly that provider, so
/// there is no separate, silently-ignored `model` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub id: ProbeId,
    pub name: String,
    pub provider: ProviderSpec,
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

/// A complete, self-describing provider target for a probe.
///
/// Every variant carries the values the *user* chooses (the model/deployment,
/// and any instance address such as the Ollama base URL). Deployment-wide
/// infrastructure (base URLs, api-version, embedding model/dim, the Azure
/// resource endpoint) comes from `config.providers.*`; the secret API key comes
/// from the vault, keyed by [`ProviderSpec::provider_id`]. The runtime resolves
/// a provider deterministically from this spec — no field is ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderSpec {
    OpenAi {
        model: String,
    },
    Anthropic {
        model: String,
    },
    Ollama {
        model: String,
        base_url: String,
    },
    Azure {
        /// Azure deployment name for the chat model (acts as the "model").
        chat_deployment: String,
        /// Optional per-probe embedding deployment. Falls back to
        /// `config.providers.azure.embedding_deployment`; when neither is set,
        /// the probe runs completions-only (no drift detection).
        #[serde(default)]
        embedding_deployment: Option<String>,
    },
}

impl ProviderSpec {
    /// Provider-type identifier — the vault key for this provider's secret and
    /// its human-readable name. Single-sourced from [`provider`].
    #[must_use]
    pub fn provider_id(&self) -> &'static str {
        match self {
            ProviderSpec::OpenAi { .. } => provider::OPENAI,
            ProviderSpec::Anthropic { .. } => provider::ANTHROPIC,
            ProviderSpec::Ollama { .. } => provider::OLLAMA,
            ProviderSpec::Azure { .. } => provider::AZURE,
        }
    }

    /// The user-facing model / deployment name for display.
    #[must_use]
    pub fn model(&self) -> &str {
        match self {
            ProviderSpec::OpenAi { model }
            | ProviderSpec::Anthropic { model }
            | ProviderSpec::Ollama { model, .. } => model,
            ProviderSpec::Azure {
                chat_deployment, ..
            } => chat_deployment,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProbeSchedule {
    Cron { expression: String },
    EveryMinutes { minutes: u32 },
}

/// Current baseline schema version. v2 stores per-prompt **output-embedding
/// clouds** (one cloud per prompt, aggregated over one or more baseline runs)
/// for the conformal two-sample drift test. v1 baselines (prompt-embedding
/// centroid) are incompatible and must be re-captured.
pub const BASELINE_SCHEMA_VERSION: u32 = 2;

/// A frozen statistical snapshot — the reference point for drift detection.
///
/// For each prompt, stores a *cloud* of output (completion) embeddings sampled
/// over one or more baseline runs. The drift test compares each new run's output
/// for a prompt against that prompt's cloud.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineSnapshot {
    pub id: BaselineId,
    pub probe_id: ProbeId,
    pub captured_at: DateTime<Utc>,
    /// Schema version (see [`BASELINE_SCHEMA_VERSION`]). Defaulted to 0 for
    /// legacy records so they can be detected and rejected.
    #[serde(default)]
    pub schema_version: u32,
    /// Embedding model that produced the clouds (for migration & display).
    #[serde(default)]
    pub embedding_model: String,
    /// Per-prompt output-embedding clouds: `prompt_clouds[i]` is prompt `i`'s
    /// set of completion embeddings; each inner vector is one sample.
    #[serde(default)]
    pub prompt_clouds: Vec<Vec<Vec<f32>>>,
    /// Number of runs aggregated into this baseline (cloud depth driver).
    #[serde(default)]
    pub n_runs: usize,
    /// The most recent run folded into this baseline.
    pub run_id: RunId,
}

impl BaselineSnapshot {
    /// Embedding dimensionality of the stored clouds (0 if empty).
    ///
    /// Used to detect when a run was produced by a different embedding model
    /// than the baseline (e.g. `text-embedding-3-small` → `-3-large`), so drift
    /// comparison fails with actionable guidance rather than an opaque error.
    #[must_use]
    pub fn embedding_dim(&self) -> usize {
        self.prompt_clouds
            .iter()
            .flatten()
            .find(|v| !v.is_empty())
            .map_or(0, Vec::len)
    }

    /// True if this baseline uses the current schema and carries usable clouds.
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.schema_version >= BASELINE_SCHEMA_VERSION
            && self.prompt_clouds.iter().any(|c| !c.is_empty())
    }
}

/// Results of a single probe run (one full pass of all prompts).
///
/// Each prompt is sampled `samples_per_prompt` times, so `embeddings[i]` is the
/// list of output embeddings for prompt `i` (one per successful sample; possibly
/// fewer than requested if some samples failed, or empty for a failed/embedding-
/// less prompt). `completions[i]` keeps one representative completion per prompt
/// for display and expectation checks. Drawing several samples per prompt gives
/// the drift test a within-prompt distribution, so a single drifted prompt is no
/// longer rank-limited to `1/(k+1)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRun {
    pub id: RunId,
    pub probe_id: ProbeId,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    /// Per-prompt output embeddings. Stored apart from the rest of the run (in a
    /// dedicated table) and **omitted** when a run is read back as metadata
    /// (`RunStore::get` / `list_for_probe`), so it defaults to empty there; fetch
    /// it explicitly via `RunStore::embeddings`. Present in-memory during a run
    /// and when freshly constructed.
    #[serde(default)]
    pub embeddings: Vec<Vec<Vec<f32>>>,
    pub completions: Vec<String>,
    pub drift_report: Option<DriftReport>,
    pub status: RunStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    PartialFailure,
    Failed,
}

/// Calibrated statistical verdict comparing a run to its baseline.
///
/// Produced by a nonparametric two-sample test (per-prompt conformal, or pooled
/// MMD/energy). The `combined_p_value` is calibrated to a false-positive rate:
/// the run drifted (at the operator's `target_fpr`) when `combined_p_value <
/// target_fpr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub run_id: RunId,
    pub baseline_id: BaselineId,
    /// Calibrated combined p-value for the run (lower ⇒ stronger drift).
    pub combined_p_value: f32,
    /// Drift score `−log₁₀(combined_p_value)`; higher ⇒ stronger evidence.
    pub statistic: f32,
    /// Interpretable drift **magnitude**: how far the run's outputs moved, in
    /// standard deviations of the no-drift null. Unlike `statistic` (`−log₁₀ p`,
    /// which a large baseline can inflate for a trivial shift), this separates
    /// effect *size* from statistical *precision*. ~0 ⇒ within noise. Defaulted
    /// for reports persisted before this field existed.
    #[serde(default)]
    pub effect_size: f32,
    /// Target false-positive rate this report was judged against.
    pub target_fpr: f32,
    /// Test that produced the verdict (`per_prompt_conformal` / `pooled_two_sample`).
    pub method: String,
    /// Per-prompt p-value breakdown (empty in pooled mode).
    #[serde(default)]
    pub per_prompt: Vec<PromptDrift>,
    pub drift_level: DriftLevel,
    /// Human-readable interpretation of the statistical verdict.
    #[serde(default)]
    pub interpretation: String,
    pub computed_at: DateTime<Utc>,
}

/// Per-prompt entry in a [`DriftReport`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptDrift {
    pub prompt_index: usize,
    pub p_value: f32,
    pub n_baseline: usize,
    /// `true` when this prompt's baseline cloud is near-constant
    /// (deterministic/cached outputs), so its drift signal measures embedding
    /// noise rather than behaviour. Defaulted for reports predating this field.
    #[serde(default)]
    pub low_variance_baseline: bool,
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
    /// Fire when a run's calibrated `combined_p_value` is below this
    /// false-positive rate (e.g. `0.01`). Lower ⇒ fewer, stronger alerts.
    pub target_fpr: f32,
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
            provider: ProviderSpec::OpenAi {
                model: crate::constants::defaults::openai::MODEL.to_string(),
            },
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
    fn provider_spec_tagged_json_shape() {
        let p = ProviderSpec::Ollama {
            model: crate::constants::defaults::ollama::MODEL.to_string(),
            base_url: crate::constants::defaults::ollama::BASE_URL.to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"kind\":\"ollama\""));
        assert!(json.contains("base_url"));
    }

    #[test]
    fn provider_spec_accessors_report_id_and_model() {
        let spec = ProviderSpec::Azure {
            chat_deployment: "gpt-4o-prod".to_string(),
            embedding_deployment: None,
        };
        assert_eq!(spec.provider_id(), crate::constants::provider::AZURE);
        assert_eq!(spec.model(), "gpt-4o-prod");
    }

    #[test]
    fn baseline_snapshot_preserves_embedding_dims_after_roundtrip() {
        let snap = BaselineSnapshot {
            id: BaselineId::new(),
            probe_id: ProbeId::new(),
            captured_at: Utc::now(),
            schema_version: BASELINE_SCHEMA_VERSION,
            embedding_model: "text-embedding-3-small".to_string(),
            prompt_clouds: vec![vec![vec![0.1, 0.2, 0.3], vec![0.11, 0.19, 0.31]]],
            n_runs: 2,
            run_id: RunId::new(),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let snap2: BaselineSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap.prompt_clouds, snap2.prompt_clouds);
        assert_eq!(snap.embedding_dim(), 3);
        assert!(snap.is_current());
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
            embeddings: vec![vec![vec![0.1, 0.2]]],
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
            target_fpr: 0.01,
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
                combined_p_value: 0.002,
                statistic: 2.7,
                effect_size: 3.1,
                target_fpr: 0.01,
                method: crate::constants::method::PER_PROMPT_CONFORMAL.to_string(),
                per_prompt: Vec::new(),
                drift_level: DriftLevel::High,
                interpretation: String::new(),
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
