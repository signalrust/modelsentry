/**
 * Single source of truth for frontend identifier strings and provider/model
 * presets. Mirrors the backend `modelsentry_common::constants` so the two sides
 * cannot silently drift apart (provider keys, default models, the Ollama URL).
 */
import type { DriftLevel } from './types.js';

/** Provider discriminant tags (match the Rust `ProviderSpec` serde tags). */
export const PROVIDER_KIND = {
  OPEN_AI: 'open_ai',
  ANTHROPIC: 'anthropic',
  OLLAMA: 'ollama',
  AZURE: 'azure',
} as const;

export type ProviderKindTag = (typeof PROVIDER_KIND)[keyof typeof PROVIDER_KIND];

/** Human-readable provider labels for display. */
export const PROVIDER_LABELS: Record<ProviderKindTag, string> = {
  [PROVIDER_KIND.OPEN_AI]: 'OpenAI',
  [PROVIDER_KIND.ANTHROPIC]: 'Anthropic',
  [PROVIDER_KIND.OLLAMA]: 'Ollama',
  [PROVIDER_KIND.AZURE]: 'Azure OpenAI',
};

/**
 * Curated model presets per provider. An empty list means "free-text": Ollama
 * models and Azure deployments are user-defined, so they have no fixed menu.
 */
export const MODELS: Record<ProviderKindTag, readonly string[]> = {
  [PROVIDER_KIND.OPEN_AI]: ['gpt-5.5', 'gpt-5.4', 'gpt-5.4-mini', 'gpt-5.4-nano'],
  [PROVIDER_KIND.ANTHROPIC]: ['claude-opus-4-8', 'claude-sonnet-4-6', 'claude-haiku-4-5'],
  [PROVIDER_KIND.OLLAMA]: [],
  [PROVIDER_KIND.AZURE]: [],
};

/** Default model preselected when each provider is chosen. */
export const DEFAULT_MODELS: Record<ProviderKindTag, string> = {
  [PROVIDER_KIND.OPEN_AI]: 'gpt-5.4',
  [PROVIDER_KIND.ANTHROPIC]: 'claude-sonnet-4-6',
  [PROVIDER_KIND.OLLAMA]: 'llama3',
  [PROVIDER_KIND.AZURE]: '',
};

/** Sentinel `<select>` value that switches the model field to free-text entry. */
export const CUSTOM_MODEL = '__custom__';

/** Default Ollama base URL (mirrors the backend default). */
export const OLLAMA_DEFAULT_BASE_URL = 'http://localhost:11434';

/** Default cron preset offered in the new-probe form (hourly). */
export const DEFAULT_CRON = '0 * * * *';

/** `localStorage` keys used by the dashboard. */
export const STORAGE_KEYS = {
  API_KEY: 'ms-api-key',
  THEME: 'ms-theme',
} as const;

/** Drift-level severity ordering (None = 0 … Critical = 4), for sorting. */
export const DRIFT_ORDER: Record<DriftLevel, number> = {
  none: 0,
  low: 1,
  medium: 2,
  high: 3,
  critical: 4,
};
