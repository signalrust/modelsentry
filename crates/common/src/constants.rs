//! Single source of truth for the identifier-like "magic strings" shared across
//! the workspace.
//!
//! Only **stable identifiers** live here — strings that are matched, dispatched
//! on, or used as persistence/protocol keys, and that would silently break if a
//! copy in one crate drifted from a copy in another:
//!
//! - [`provider`] — registry / vault / dispatch keys for each LLM provider.
//! - [`method`] — drift-assessment method tags recorded in `DriftReport::method`.
//! - [`table`] — `redb` table names (the persistence key namespace).
//! - [`header`] — HTTP header names for the daemon's own API.
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
    /// Probe run results.
    pub const RUNS: &str = "runs";
    /// Alert rules.
    pub const ALERT_RULES: &str = "alert_rules";
    /// Fired alert events.
    pub const ALERT_EVENTS: &str = "alert_events";
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
        /// Default chat model.
        pub const MODEL: &str = "gpt-5.4";
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
        /// Default chat model.
        pub const MODEL: &str = "claude-sonnet-4-6";
        /// API base URL.
        pub const BASE_URL: &str = "https://api.anthropic.com";
        /// Messages API version header value.
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
        /// Default chat model.
        pub const MODEL: &str = "llama3";
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
        /// Default `api-version` query parameter for the Azure OpenAI REST API.
        pub const API_VERSION: &str = "2024-10-21";
        /// Native output dimension of the default embedding deployment
        /// (`text-embedding-3-small`). Override to match your deployment.
        pub const EMBEDDING_DIM: usize = 1536;
        /// Per-request HTTP timeout (seconds).
        pub const TIMEOUT_SECS: u64 = 30;
        /// Auth request header name (vs OpenAI's `Authorization: Bearer`).
        pub const API_KEY_HEADER: &str = "api-key";
    }
}
