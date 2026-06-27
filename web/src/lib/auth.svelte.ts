/**
 * Reactive API-key store for authenticated daemon access.
 *
 * The daemon serves a pre-built static bundle, so an API key cannot be injected
 * at build time for end users (the `VITE_API_KEY` env var only helps local dev
 * builds). Instead the key is entered at runtime through the dashboard and
 * persisted in `localStorage`. The {@link api} client reads {@link auth.key} on
 * every request and calls {@link auth.markUnauthorized} when the daemon rejects
 * a request with `401`, which the UI observes to prompt for a key.
 */
import { browser } from '$app/environment';
import { STORAGE_KEYS } from './constants.js';

const STORAGE_KEY = STORAGE_KEYS.API_KEY;

function readStored(): string | null {
  if (!browser) return null;
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

let key = $state<string | null>(readStored());
let unauthorized = $state(false);

export const auth = {
  /** Current API key, or `null` when none is configured. */
  get key(): string | null {
    return key;
  },

  /** `true` once the daemon has rejected a request with `401 Unauthorized`. */
  get unauthorized(): boolean {
    return unauthorized;
  },

  /** Persist a new key (or clear it when blank/null) and reset the 401 flag. */
  set(newKey: string | null): void {
    const trimmed = newKey?.trim() ?? '';
    key = trimmed === '' ? null : trimmed;
    unauthorized = false;
    if (!browser) return;
    try {
      if (key === null) localStorage.removeItem(STORAGE_KEY);
      else localStorage.setItem(STORAGE_KEY, key);
    } catch {
      // localStorage unavailable (private mode / disabled) — key stays in memory
    }
  },

  /** Flag that the daemon rejected the current key. Called by the API client. */
  markUnauthorized(): void {
    unauthorized = true;
  },
};
