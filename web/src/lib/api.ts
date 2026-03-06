/**
 * Typed REST API client for the ModelSentry daemon.
 *
 * All responses are validated at runtime with zod.
 * Non-2xx responses throw an `ApiError` with the error message from the server.
 */
import { z } from 'zod';
import type {
  AlertEvent,
  AlertRule,
  BaselineSnapshot,
  CreateAlertRuleRequest,
  CreateProbeRequest,
  Probe,
  ProbeRun,
} from './types.js';

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/** Base URL of the daemon API. Override via VITE_API_URL env variable. */
const BASE_URL: string =
  import.meta.env.VITE_API_URL ?? 'http://localhost:7740/api';

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

// ---------------------------------------------------------------------------
// Zod schemas — mirror web/src/lib/types.ts shapes
// ---------------------------------------------------------------------------

const providerKindSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('open_ai') }),
  z.object({ kind: z.literal('anthropic') }),
  z.object({ kind: z.literal('ollama'), base_url: z.string() }),
  z.object({ kind: z.literal('azure_open_ai'), endpoint: z.string(), deployment: z.string() }),
]);

const probeScheduleSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('cron'), expression: z.string() }),
  z.object({ kind: z.literal('every_minutes'), minutes: z.number() }),
]);

const alertChannelSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('webhook'), url: z.string() }),
  z.object({ kind: z.literal('slack'), webhook_url: z.string() }),
  z.object({ kind: z.literal('email'), address: z.string() }),
]);

const probePromptSchema = z.object({
  id: z.string(),
  text: z.string(),
  expected_contains: z.string().nullable(),
  expected_not_contains: z.string().nullable(),
});

const probeSchema = z.object({
  id: z.string(),
  name: z.string(),
  provider: providerKindSchema,
  model: z.string(),
  prompts: z.array(probePromptSchema),
  schedule: probeScheduleSchema,
  created_at: z.string(),
  updated_at: z.string(),
});

const driftLevelSchema = z.enum(['none', 'low', 'medium', 'high', 'critical']);

const driftReportSchema = z.object({
  run_id: z.string(),
  baseline_id: z.string(),
  kl_divergence: z.number(),
  cosine_distance: z.number(),
  output_entropy_delta: z.number(),
  drift_level: driftLevelSchema,
  computed_at: z.string(),
});

const probeRunSchema = z.object({
  id: z.string(),
  probe_id: z.string(),
  started_at: z.string(),
  finished_at: z.string(),
  embeddings: z.array(z.array(z.number())),
  completions: z.array(z.string()),
  drift_report: driftReportSchema.nullable(),
  status: z.enum(['success', 'partial_failure', 'failed']),
});

const baselineSnapshotSchema = z.object({
  id: z.string(),
  probe_id: z.string(),
  captured_at: z.string(),
  embedding_centroid: z.array(z.number()),
  embedding_variance: z.number(),
  output_tokens: z.array(z.array(z.string())),
  run_id: z.string(),
});

const alertRuleSchema = z.object({
  id: z.string(),
  probe_id: z.string(),
  kl_threshold: z.number(),
  cosine_threshold: z.number(),
  channels: z.array(alertChannelSchema),
  active: z.boolean(),
});

const alertEventSchema = z.object({
  id: z.string(),
  rule_id: z.string(),
  drift_report: driftReportSchema,
  fired_at: z.string(),
  acknowledged: z.boolean(),
});

// ---------------------------------------------------------------------------
// Fetch helper
// ---------------------------------------------------------------------------

async function request<T>(
  schema: z.ZodType<T>,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const res = await fetch(`${BASE_URL}${path}`, {
    headers: { 'Content-Type': 'application/json', ...init?.headers },
    ...init,
  });

  if (!res.ok) {
    let message = res.statusText;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // ignore parse errors — use statusText
    }
    throw new ApiError(res.status, message);
  }

  // 204 No Content — return undefined cast to T
  if (res.status === 204) {
    return undefined as unknown as T;
  }

  const json: unknown = await res.json();
  return schema.parse(json);
}

// ---------------------------------------------------------------------------
// Public API surface
// ---------------------------------------------------------------------------

export const api = {
  probes: {
    list: (): Promise<Probe[]> =>
      request(z.array(probeSchema), '/probes'),

    create: (body: CreateProbeRequest): Promise<Probe> =>
      request(probeSchema, '/probes', {
        method: 'POST',
        body: JSON.stringify(body),
      }),

    get: (id: string): Promise<Probe> =>
      request(probeSchema, `/probes/${id}`),

    delete: (id: string): Promise<void> =>
      request(z.undefined(), `/probes/${id}`, { method: 'DELETE' }),

    runNow: (id: string): Promise<ProbeRun> =>
      request(probeRunSchema, `/probes/${id}/run-now`, { method: 'POST' }),
  },

  baselines: {
    listForProbe: (probeId: string): Promise<BaselineSnapshot[]> =>
      request(z.array(baselineSnapshotSchema), `/probes/${probeId}/baselines`),

    captureForProbe: (probeId: string): Promise<BaselineSnapshot> =>
      request(baselineSnapshotSchema, `/probes/${probeId}/baselines`, { method: 'POST' }),

    getLatestForProbe: (probeId: string): Promise<BaselineSnapshot> =>
      request(baselineSnapshotSchema, `/probes/${probeId}/baselines/latest`),
  },

  runs: {
    listForProbe: (probeId: string, limit = 20): Promise<ProbeRun[]> =>
      request(z.array(probeRunSchema), `/probes/${probeId}/runs?limit=${limit}`),

    get: (runId: string): Promise<ProbeRun> =>
      request(probeRunSchema, `/runs/${runId}`),
  },

  alerts: {
    listRulesForProbe: (probeId: string): Promise<AlertRule[]> =>
      request(z.array(alertRuleSchema), `/probes/${probeId}/alerts`),

    createRuleForProbe: (probeId: string, body: CreateAlertRuleRequest): Promise<AlertRule> =>
      request(alertRuleSchema, `/probes/${probeId}/alerts`, {
        method: 'POST',
        body: JSON.stringify(body),
      }),

    deleteRule: (ruleId: string): Promise<void> =>
      request(z.undefined(), `/alerts/${ruleId}`, { method: 'DELETE' }),

    listEvents: (limit = 50): Promise<AlertEvent[]> =>
      request(z.array(alertEventSchema), `/events?limit=${limit}`),

    acknowledgeEvent: (id: string): Promise<void> =>
      request(z.undefined(), `/events/${id}/acknowledge`, { method: 'POST' }),
  },
};
