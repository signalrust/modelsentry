/**
 * TypeScript mirror of the Rust `modelsentry-common` models.
 * All DateTime fields are ISO-8601 strings as serialized by `chrono`.
 */

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

export type DriftLevel = 'none' | 'low' | 'medium' | 'high' | 'critical';

export type RunStatus = 'success' | 'partial_failure' | 'failed';

// ---------------------------------------------------------------------------
// Provider / Schedule discriminated unions (match Rust serde tag)
// ---------------------------------------------------------------------------

export type ProviderKind =
  | { kind: 'open_ai' }
  | { kind: 'anthropic' }
  | { kind: 'ollama'; base_url: string }
  | { kind: 'azure_open_ai'; endpoint: string; deployment: string };

export type ProbeSchedule =
  | { kind: 'cron'; expression: string }
  | { kind: 'every_minutes'; minutes: number };

export type AlertChannel =
  | { kind: 'webhook'; url: string }
  | { kind: 'slack'; webhook_url: string }
  | { kind: 'email'; address: string };

// ---------------------------------------------------------------------------
// Core models
// ---------------------------------------------------------------------------

export interface ProbePrompt {
  id: string;
  text: string;
  expected_contains: string | null;
  expected_not_contains: string | null;
}

export interface Probe {
  id: string;
  name: string;
  provider: ProviderKind;
  model: string;
  prompts: ProbePrompt[];
  schedule: ProbeSchedule;
  created_at: string;
  updated_at: string;
}

export interface DriftReport {
  run_id: string;
  baseline_id: string;
  kl_divergence: number;
  cosine_distance: number;
  output_entropy_delta: number;
  drift_level: DriftLevel;
  computed_at: string;
}

export interface ProbeRun {
  id: string;
  probe_id: string;
  started_at: string;
  finished_at: string;
  embeddings: number[][];
  completions: string[];
  drift_report: DriftReport | null;
  status: RunStatus;
}

export interface BaselineSnapshot {
  id: string;
  probe_id: string;
  captured_at: string;
  embedding_centroid: number[];
  embedding_variance: number;
  output_tokens: string[][];
  run_id: string;
}

export interface AlertRule {
  id: string;
  probe_id: string;
  kl_threshold: number;
  cosine_threshold: number;
  channels: AlertChannel[];
  active: boolean;
}

export interface AlertEvent {
  id: string;
  rule_id: string;
  drift_report: DriftReport;
  fired_at: string;
  acknowledged: boolean;
}

// ---------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------

export interface CreateProbeRequest {
  name: string;
  provider: ProviderKind;
  model: string;
  prompts: ProbePrompt[];
  schedule: ProbeSchedule;
}

export interface CreateAlertRuleRequest {
  kl_threshold: number;
  cosine_threshold: number;
  channels: AlertChannel[];
  active?: boolean;
}
