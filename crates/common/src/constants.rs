//! Single application-wide source of truth for compile-time constants.
//!
//! Every constant the workspace shares or tunes lives here, grouped by concern,
//! so a value is defined once and never redeclared (or silently diverged) across
//! crates:
//!
//! - [`provider`] — registry / vault / dispatch keys for each LLM provider.
//! - [`credential`] — vault keys for secrets that are not provider API keys.
//! - [`method`] — drift-assessment method tags recorded in `DriftReport::method`.
//! - [`table`] — `redb` table names (the persistence key namespace).
//! - [`server`] — daemon HTTP-server limits/defaults (timeouts, body cap, rate
//!   limit, CORS default).
//! - [`header`] — HTTP header names for the daemon's own API.
//! - [`defaults`] — provider model IDs, base URLs, embedding dims, timeouts.
//! - [`drift`] — numeric floors/tolerances shared by the drift algorithms.
//! - [`alerts`] — alerting defaults (target FPR, capture depth, cooldown, SMTP).
//!
//! Human-readable text (error messages, log lines, UI copy) is intentionally
//! **not** centralized here: it belongs next to the code that emits it. Likewise,
//! serde tag values (e.g. the `"anthropic"` in a `ProviderSpec` JSON payload) are
//! owned by the `#[serde(rename_all)]` derive, not by this module.

/// Provider identifiers — the key under which each provider is stored in the
/// vault and registered in the runtime provider registry, and the value its
/// `provider_name()` returns.
// "OpenAI" is a brand name, not a code item — exempt from the backtick lint.
#[allow(clippy::doc_markdown)]
pub mod provider {
    /// OpenAI (chat + embeddings).
    pub const OPENAI: &str = "openai";
    /// Anthropic (chat only).
    pub const ANTHROPIC: &str = "anthropic";
    /// Ollama (local; chat + embeddings). Registry keys are namespaced as
    /// `ollama:<base_url>`.
    pub const OLLAMA: &str = "ollama";
    /// Azure OpenAI. Registry keys are namespaced as
    /// `azure:<endpoint>:<deployment>`.
    pub const AZURE: &str = "azure";
}

/// Vault keys for secrets that are not provider API keys.
pub mod credential {
    /// Vault key under which the SMTP password for the email alert channel is
    /// stored (the rest of the SMTP settings are in `[alerts.smtp]` config).
    pub const SMTP_PASSWORD: &str = "smtp";
}

/// Drift-assessment method tags, recorded in `DriftReport::method` so a report
/// records which test produced its verdict.
pub mod method {
    /// Per-prompt conformal test (Šidák-combined). The preferred, higher-power
    /// mode used when baselines carry ≥2 samples per prompt.
    pub const PER_PROMPT_CONFORMAL: &str = "per_prompt_conformal";
    /// Pooled MMD/energy permutation test — the single-sample-baseline fallback.
    pub const POOLED_TWO_SAMPLE: &str = "pooled_two_sample";
}

/// `redb` table names. The `redb` `TableDefinition` values live in the store
/// crate (they carry a `redb` type), but the *names* are sourced from here so a
/// table is never opened under two slightly different spellings.
pub mod table {
    /// Configured probes.
    pub const PROBES: &str = "probes";
    /// Captured baseline snapshots.
    pub const BASELINES: &str = "baselines";
    /// Probe run **metadata** (everything except the heavy embeddings): status,
    /// timestamps, completions, drift report. Keyed by run id. The bulk
    /// embeddings live in [`RUN_EMBEDDINGS`] so listing runs never decodes them.
    pub const RUNS: &str = "runs";
    /// Per-run output embeddings, keyed by run id. Read only by baseline capture,
    /// which aggregates the runs it folds into a cloud — kept out of [`RUNS`] so
    /// a dashboard run-list does not pay to decode megabytes of vectors.
    pub const RUN_EMBEDDINGS: &str = "run_embeddings";
    /// Time-ordered index over runs, keyed `{probe_id}|{rev_ts}|{run_id}` so the
    /// most-recent N runs for a probe are a bounded range scan instead of a
    /// full-table scan.
    pub const RUN_INDEX: &str = "run_index";
    /// Alert rules.
    pub const ALERT_RULES: &str = "alert_rules";
    /// Fired alert events.
    pub const ALERT_EVENTS: &str = "alert_events";
    /// Per-rule most-recent fire time, maintained on each event insert so the
    /// cooldown check (`last_fired_for_rule`, consulted every scheduled run) is
    /// an O(1) point lookup instead of a full scan of [`ALERT_EVENTS`].
    pub const ALERT_LAST_FIRED: &str = "alert_last_fired";
    /// Per-probe scheduler state (the next scheduled run time), so the daemon
    /// resumes each probe's cadence across restarts instead of re-phasing it.
    pub const SCHEDULE_STATE: &str = "schedule_state";
    /// Per-rule alpha-spend ledger backing the rolling-window sequential
    /// control: one entry per look, summed within the window to enforce the
    /// false-alarm budget.
    pub const ALERT_SPEND: &str = "alert_spend";
}

/// HTTP server limits and defaults for the daemon's own API.
#[allow(clippy::doc_markdown)]
pub mod server {
    /// Default request timeout (seconds) when `[server] timeout_secs` is unset.
    pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
    /// Default allowed CORS origin (the Vite dev server) when `[server]
    /// cors_origin` is unset. `"*"` allows all origins (not recommended for
    /// production).
    pub const DEFAULT_CORS_ORIGIN: &str = "http://localhost:5173";
    /// Maximum accepted request body. Probe/alert payloads are small; anything
    /// larger is rejected with `413 Payload Too Large` to bound memory use.
    pub const MAX_BODY_BYTES: usize = 1024 * 1024; // 1 MiB
    /// Per-client rate-limit bucket capacity (keyed by peer IP): the bucket
    /// holds up to this many requests, refilling one token every
    /// [`RATE_LIMIT_REPLENISH_SECS`] seconds — throttling credential
    /// brute-forcing while allowing a steady stream.
    pub const RATE_LIMIT_BURST: u32 = 100;
    /// Token refill period (seconds) for the per-client rate limiter.
    pub const RATE_LIMIT_REPLENISH_SECS: u64 = 1;
}

/// HTTP header names for the daemon's own API.
pub mod header {
    /// API-key auth header accepted as an alternative to
    /// `Authorization: Bearer <key>`. (Distinct from any header an *upstream*
    /// provider API may require.)
    pub const API_KEY: &str = "x-api-key";
}

/// Default provider settings — the *empirical* layer (model IDs, base URLs,
/// embedding dimensions). Single-sourced here so the config-deserialization
/// defaults in `config.rs` and the standalone provider constructors in
/// `modelsentry-core` cannot drift apart. These genuinely date over time; update
/// them here and both layers follow.
#[allow(clippy::doc_markdown)]
pub mod defaults {
    /// Default completion-length cap shared by all chat providers.
    pub const MAX_TOKENS: u32 = 1024;

    /// OpenAI defaults.
    pub mod openai {
        /// Current GA chat models, most-capable first — the menu the dashboard
        /// offers and the set tests draw from (June 2026: `gpt-5.5` flagship,
        /// `gpt-5.4`, `gpt-5.4-mini`). Models date over time; update the list
        /// here.
        pub const MODELS: [&str; 3] = ["gpt-5.5", "gpt-5.4", "gpt-5.4-mini"];
        /// Default chat model — the balanced `gpt-5.4` (second of [`MODELS`]),
        /// not the flagship, since probes call it repeatedly on a schedule.
        pub const MODEL: &str = MODELS[1];
        /// Default embedding model (1536-dimensional).
        pub const EMBEDDING_MODEL: &str = "text-embedding-3-small";
        /// Native output dimension of [`EMBEDDING_MODEL`].
        pub const EMBEDDING_DIM: usize = 1536;
        /// API base URL.
        pub const BASE_URL: &str = "https://api.openai.com";
        /// Per-request HTTP timeout (seconds).
        pub const TIMEOUT_SECS: u64 = 30;
    }

    /// Anthropic defaults.
    pub mod anthropic {
        /// Current Claude 4.X family chat models, most-capable first — the menu
        /// the dashboard offers and the set tests draw from (June 2026: Opus
        /// 4.8 / Sonnet 4.6 / Haiku 4.5; Sonnet 4 & Opus 4 retired 2026-06-15).
        /// Model IDs are dateless pinned snapshots from the 4.6 generation on.
        /// Update the list here as the family advances.
        pub const MODELS: [&str; 3] = ["claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5"];
        /// Default chat model — the balanced `claude-sonnet-4-6` (second of
        /// [`MODELS`]), not the flagship, since probes call it on a schedule.
        pub const MODEL: &str = MODELS[1];
        /// API base URL.
        pub const BASE_URL: &str = "https://api.anthropic.com";
        /// Messages API version header value. This is a stable API *contract*
        /// identifier (not a freshness date): `2023-06-01` is the value current
        /// Claude models still require in 2026 — bump only when Anthropic
        /// publishes a new versioned contract.
        pub const API_VERSION: &str = "2023-06-01";
        /// Per-request HTTP timeout (seconds).
        pub const TIMEOUT_SECS: u64 = 30;
        /// Auth request header name (vs OpenAI's `Authorization: Bearer`).
        pub const API_KEY_HEADER: &str = "x-api-key";
        /// API-version request header name.
        pub const VERSION_HEADER: &str = "anthropic-version";
    }

    /// Ollama (local) defaults.
    pub mod ollama {
        /// Current local chat models, most-preferred first — the menu the
        /// dashboard offers and the set tests draw from (June 2026 Ollama pull
        /// tags: `llama4`, `qwen3`, `gemma4`). The default [`MODEL`] is the
        /// first entry. Update the list here as the library advances.
        pub const MODELS: [&str; 3] = ["llama4", "qwen3", "gemma4"];
        /// Default chat model (first of [`MODELS`]).
        pub const MODEL: &str = MODELS[0];
        /// API base URL.
        pub const BASE_URL: &str = "http://localhost:11434";
        /// Nominal embedding dimension (model-dependent; a conservative default
        /// covering popular embedding models). Used only for the
        /// has-embeddings capability check, not as ground truth.
        pub const EMBEDDING_DIM: usize = 1024;
        /// Per-request HTTP timeout (seconds). Longer than the cloud providers
        /// because local first-run generation on CPU can be slow.
        pub const TIMEOUT_SECS: u64 = 120;
    }

    /// Azure OpenAI defaults. The resource `endpoint` and deployment names are
    /// deployment-specific and have no universal default — they are supplied via
    /// `[providers.azure]` config and the per-probe spec.
    pub mod azure {
        /// Known underlying chat models behind Azure deployments — Azure hosts
        /// the same OpenAI models, addressed by a user-named *deployment* (so
        /// there is no default model name). The API still reports the underlying
        /// model identity in each response; these are the values that field
        /// carries. Sourced from [`super::openai::MODELS`] so the two never
        /// diverge.
        pub const MODELS: [&str; 3] = super::openai::MODELS;
        /// `api-version` token for the Azure OpenAI **v1 API** (the current
        /// generation, GA since Aug 2025). The v1 surface replaced the old
        /// monthly *dated* `api-version` parameters with a single stable
        /// `preview` channel that always exposes the latest features, so there
        /// is no year to keep current here. Used on the `/openai/v1/{op}`
        /// routes (the deployment is passed as `model` in the request body).
        /// Operator-overridable via `[providers.azure]` to pin a dated version.
        pub const API_VERSION: &str = "preview";
        /// Native output dimension of the default embedding deployment
        /// (`text-embedding-3-small`). Override to match your deployment.
        pub const EMBEDDING_DIM: usize = 1536;
        /// Per-request HTTP timeout (seconds).
        pub const TIMEOUT_SECS: u64 = 30;
        /// Auth request header name (vs OpenAI's `Authorization: Bearer`).
        pub const API_KEY_HEADER: &str = "api-key";
    }
}

/// Numeric floors and tolerances shared by the drift algorithms (two-sample,
/// conformal, and stratified-permutation paths). Centralized so the same
/// guard value cannot diverge between `twosample` and `assessment`.
pub mod drift {
    /// Minimum samples per group for the unbiased MMD² estimator
    /// (`1 / (m (m − 1))` needs `m ≥ 2`).
    pub const MIN_SAMPLES_PER_GROUP: usize = 2;

    /// Floor applied to the RBF bandwidth so identical pooled points (median
    /// pairwise distance 0) do not produce a degenerate kernel.
    pub const BANDWIDTH_FLOOR: f32 = 1e-6;

    /// Floor on a standard deviation before dividing by it when standardizing
    /// scores/statistics, so a near-deterministic (temperature-0 / cached)
    /// baseline cloud does not divide by ~0.
    pub const STD_FLOOR: f32 = 1e-6;

    /// Tolerance when counting permutation statistics `≥` the observed value.
    pub const PERMUTATION_TOLERANCE: f32 = 1e-6;

    /// Minimum RMS spread (cloud radius) a baseline cloud must have for its
    /// drift verdict to be meaningful. Below this the points are effectively
    /// identical (temperature-0 / cached / deterministic outputs), so the test
    /// measures embedding noise rather than behavioural drift — the report flags
    /// such prompts so the operator does not trust a spurious signal. Embeddings
    /// here are O(0.1–1) apart, so `1e-3` flags only genuine degeneracy.
    pub const BASELINE_MIN_CLOUD_SPREAD: f32 = 1e-3;
}

/// Alerting defaults — the fallback values for `[alerts]` config and the alert
/// engine. Operator-overridable in `config/default.toml`; these are the
/// single-sourced defaults the deserializer falls back to.
pub mod alerts {
    /// Default calibrated false-positive rate a run is alerted at.
    pub const TARGET_FPR: f32 = 0.01;

    /// Default number of recent successful runs aggregated into a baseline.
    pub const BASELINE_CAPTURE_RUNS: usize = 20;

    /// Default permutations for the pooled-fallback two-sample test.
    pub const PERMUTATIONS: usize = 200;

    /// Default completions sampled per prompt on each run.
    pub const SAMPLES_PER_PROMPT: usize = 3;

    /// Default minimum seconds between alert notifications for one rule
    /// (de-duplication / cooldown window). One hour. `0` disables.
    pub const COOLDOWN_SECS: u64 = 3600;

    /// Default rolling window for the sequential-control / alpha-spending
    /// false-alarm budget, in seconds. Thirty days. Only consulted when
    /// `[alerts.sequential]` is present.
    pub const SEQUENTIAL_WINDOW_SECS: u64 = 2_592_000;

    /// Default per-rule alpha budget spent over one [`SEQUENTIAL_WINDOW_SECS`]
    /// window: the bound on the **expected number of false alarms** per rule
    /// per window. Only consulted when `[alerts.sequential]` is present; `0`
    /// disables the control even when the block is present.
    pub const SEQUENTIAL_ALPHA_BUDGET: f32 = 0.05;

    /// Default SMTP submission port (RFC 6409 STARTTLS submission).
    pub const SMTP_PORT: u16 = 587;

    /// HTTP timeout (seconds) for outbound alert/webhook delivery — short, so a
    /// slow or hung receiver cannot stall the alert path.
    pub const WEBHOOK_TIMEOUT_SECS: u64 = 10;
}

/// Scheduler / probe-execution tuning.
pub mod scheduler {
    use std::time::Duration;

    /// Default cap on the number of probe runs executing concurrently across all
    /// probes. Each run may itself fan out up to the per-run prompt concurrency,
    /// so this bounds the total outbound load a fleet of probes puts on one
    /// provider (avoiding a restart/​reconcile stampede). Must be ≥ 1.
    pub const MAX_CONCURRENT_RUNS: usize = 8;

    /// How often the scheduler re-reads the probe set from the store to pick up
    /// probes added, edited, or deleted via the API/CLI after startup.
    pub const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

    /// Maximum number of prompts a single probe run executes concurrently.
    pub const PROBE_CONCURRENCY: usize = 4;
}
