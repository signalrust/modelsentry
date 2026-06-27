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

export type ProviderSpec =
  | { kind: 'open_ai'; model: string }
  | { kind: 'anthropic'; model: string }
  | { kind: 'ollama'; model: string; base_url: string }
  | { kind: 'azure'; chat_deployment: string; embedding_deployment?: string | null };

/** User-facing model / deployment name for a spec (mirrors `ProviderSpec::model`). */
export function providerModel(spec: ProviderSpec): string {
  return spec.kind === 'azure' ? spec.chat_deployment : spec.model;
}

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
  provider: ProviderSpec;
  prompts: ProbePrompt[];
  schedule: ProbeSchedule;
  created_at: string;
  updated_at: string;
}

export interface PromptDrift {
  prompt_index: number;
  p_value: number;
  n_baseline: number;
}

export interface DriftReport {
  run_id: string;
  baseline_id: string;
  /** Calibrated combined p-value for the run (lower ⇒ stronger drift). */
  combined_p_value: number;
  /** Drift score = −log₁₀(combined_p_value); higher ⇒ stronger evidence. */
  statistic: number;
  /** Target false-positive rate this report was judged against. */
  target_fpr: number;
  /** Test that produced the verdict (`per_prompt_conformal` / `pooled_two_sample`). */
  method: string;
  /** Per-prompt p-value breakdown (empty in pooled mode). */
  per_prompt: PromptDrift[];
  drift_level: DriftLevel;
  /** Human-readable interpretation of the statistical verdict. */
  interpretation: string;
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
  /** Schema version; v2 stores per-prompt output-embedding clouds. */
  schema_version: number;
  /** Embedding model that produced the clouds. */
  embedding_model: string;
  /** Per-prompt output-embedding clouds: prompt_clouds[i] is prompt i's samples. */
  prompt_clouds: number[][][];
  /** Number of runs aggregated into this baseline. */
  n_runs: number;
  run_id: string;
}

export interface AlertRule {
  id: string;
  probe_id: string;
  /** Fire when a run's combined p-value falls below this false-positive rate. */
  target_fpr: number;
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
  provider: ProviderSpec;
  prompts: ProbePrompt[];
  schedule: ProbeSchedule;
}

export interface CreateAlertRuleRequest {
  target_fpr: number;
  channels: AlertChannel[];
  active?: boolean;
}
