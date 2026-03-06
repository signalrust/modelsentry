# ModelSentry — Project Plan
**Format:** Phase → Task → Files → Definition of Done → Signatures  
**Date:** March 6, 2026

Each task is sized to complete in one focused session (2–4 hours).  
Complete tasks in order within a phase. Phases 0–3 have no external dependencies (no LLM keys needed to run tests).

---

## Phase 0 — Scaffold

### Task 0.1 — Workspace Skeleton

**Description:** Create the Cargo workspace root and all crate skeletons with correct `Cargo.toml` files and empty `src/lib.rs` / `src/main.rs`. No logic yet — just ensure `cargo check --workspace` passes.

**Files to Create:**
```
modelsentry/Cargo.toml
modelsentry/rust-toolchain.toml
modelsentry/.rustfmt.toml
modelsentry/.clippy.toml
modelsentry/Makefile
modelsentry/crates/common/Cargo.toml
modelsentry/crates/common/src/lib.rs
modelsentry/crates/core/Cargo.toml
modelsentry/crates/core/src/lib.rs
modelsentry/crates/store/Cargo.toml
modelsentry/crates/store/src/lib.rs
modelsentry/crates/daemon/Cargo.toml
modelsentry/crates/daemon/src/main.rs
modelsentry/crates/cli/Cargo.toml
modelsentry/crates/cli/src/main.rs
```

**Definition of Done:**
- [ ] `cargo check --workspace` exits 0
- [ ] `cargo fmt --all -- --check` exits 0
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] All crate names in workspace resolver 2

---

### Task 0.2 — CI Pipeline

**Description:** Add GitHub Actions CI workflow. Must run on every push and PR.

**Files to Create:**
```
modelsentry/.github/workflows/ci.yml
```

**Definition of Done:**
- [ ] Workflow triggers on `push` and `pull_request`
- [ ] Steps: `fmt-check` → `clippy` → `nextest run` → `cargo audit`
- [ ] Uses `Swatinem/rust-cache@v2` for dependency caching
- [ ] Passes on a clean repo (no warnings, no audit findings in empty crates)

---

### Task 0.3 — Frontend Scaffold

**Description:** Initialize SvelteKit project with Tailwind and the directory structure defined in ARCHITECTURE.md.

**Files to Create:**
```
modelsentry/web/package.json
modelsentry/web/svelte.config.js
modelsentry/web/vite.config.ts
modelsentry/web/tailwind.config.ts
modelsentry/web/tsconfig.json
modelsentry/web/src/app.html
modelsentry/web/src/app.css
modelsentry/web/src/lib/api.ts          (empty stubs)
modelsentry/web/src/lib/types.ts        (empty)
modelsentry/web/src/routes/+layout.svelte
modelsentry/web/src/routes/+page.svelte
```

**Definition of Done:**
- [ ] `npm run dev` starts SvelteKit dev server on port 5173
- [ ] `npm run build` produces static output without errors
- [ ] Tailwind classes render correctly (test with one `text-red-500` element)

---

## Phase 1 — Common Types

### Task 1.1 — Newtype IDs

**Description:** Define all strongly-typed ID types. These are the foundation — every other crate depends on them.

**Files to Create/Modify:**
```
crates/common/src/types.rs   (create)
crates/common/src/lib.rs     (modify: pub mod types)
```

**Method Signatures:**
```rust
// crates/common/src/types.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProbeId(Uuid);
impl ProbeId {
    pub fn new() -> Self;
    pub fn from_uuid(id: Uuid) -> Self;
}
impl fmt::Display for ProbeId { ... }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BaselineId(Uuid);
impl BaselineId {
    pub fn new() -> Self;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(Uuid);
impl RunId {
    pub fn new() -> Self;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AlertRuleId(Uuid);
impl AlertRuleId {
    pub fn new() -> Self;
}

/// Newtype over String. Never implements Display or Debug that shows the value.
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKey(Secret<String>);
impl ApiKey {
    pub fn new(raw: String) -> Self;
    pub fn expose(&self) -> &str;
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn probe_id_new_is_unique();            // two calls produce different IDs
    #[test]
    fn probe_id_display_is_uuid_string();   // Display outputs UUID format
    #[test]
    fn probe_id_roundtrip_json();           // serde serialize then deserialize equals original
    #[test]
    fn api_key_debug_does_not_expose_secret(); // format!("{:?}", key) must not contain raw value
}
```

**Definition of Done:**
- [ ] All ID types compile with `Serialize`/`Deserialize`
- [ ] `ApiKey` debug/display output confirmed to not expose raw key value (test asserts on string content)
- [ ] All tests pass

---

### Task 1.2 — Error Types

**Description:** Define the domain error hierarchy using `thiserror`. All library crates use these types.

**Files to Create/Modify:**
```
crates/common/src/error.rs   (create)
crates/common/src/lib.rs     (modify: pub mod error)
```

**Method Signatures:**
```rust
// crates/common/src/error.rs

#[derive(Debug, thiserror::Error)]
pub enum ModelSentryError {
    #[error("provider error: {message}")]
    Provider { message: String },

    #[error("provider returned HTTP {status}: {body}")]
    ProviderHttp { status: u16, body: String },

    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("empty embedding vector")]
    EmptyEmbedding,

    #[error("baseline not found: {id}")]
    BaselineNotFound { id: String },

    #[error("probe not found: {id}")]
    ProbeNotFound { id: String },

    #[error("storage error: {0}")]
    Storage(#[from] redb::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("vault error: {message}")]
    Vault { message: String },

    #[error("configuration error: {message}")]
    Config { message: String },
}

pub type Result<T> = std::result::Result<T, ModelSentryError>;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn error_display_does_not_include_secrets();   // Provider error with key-like message checked
    #[test]
    fn dimension_mismatch_error_includes_sizes();  // format string includes both numbers
    #[test]
    fn result_alias_is_model_sentry_error();       // type check via compile
}
```

**Definition of Done:**
- [ ] All variants compile and have correct `#[from]` impls where applicable
- [ ] `cargo doc --no-deps` generates docs without warnings
- [ ] All tests pass

---

### Task 1.3 — Domain Models

**Description:** Define the core domain structs: `Probe`, `ProbePrompt`, `BaselineSnapshot`, `ProbeRun`, `DriftReport`, `AlertRule`, `AlertEvent`.

**Files to Create/Modify:**
```
crates/common/src/models.rs   (create)
crates/common/src/lib.rs      (modify: pub mod models)
```

**Method Signatures:**
```rust
// crates/common/src/models.rs

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
    pub expected_contains: Option<String>,  // optional string match check
    pub expected_not_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Ollama { base_url: String },
    AzureOpenAi { endpoint: String, deployment: String },
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
    pub embedding_centroid: Vec<f32>,    // mean across all probe embeddings
    pub embedding_variance: f32,         // average pairwise variance
    pub output_tokens: Vec<Vec<String>>, // tokenized outputs per prompt (for entropy)
    pub run_id: RunId,                   // the run this snapshot was computed from
}

/// Results of a single probe run (one full pass of all prompts)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRun {
    pub id: RunId,
    pub probe_id: ProbeId,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub embeddings: Vec<Vec<f32>>,       // one embedding per prompt
    pub completions: Vec<String>,        // one raw completion per prompt
    pub drift_report: Option<DriftReport>,
    pub status: RunStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus { Success, PartialFailure, Failed }

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftLevel { None, Low, Medium, High, Critical }

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
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn probe_serializes_and_deserializes_round_trip();
    #[test]
    fn drift_level_ordering_is_correct();   // None < Low < Medium < High < Critical
    #[test]
    fn provider_kind_tagged_json_shape();   // verify tag + content serde shape
    #[test]
    fn baseline_snapshot_preserves_embedding_dims_after_roundtrip();
}
```

**Definition of Done:**
- [ ] All models compile with `Serialize`/`Deserialize`
- [ ] Round-trip JSON test passes for every top-level struct
- [ ] `cargo doc` clean

---

### Task 1.4 — App Config

**Description:** Define `AppConfig` struct that deserializes from TOML config file. Validate on load.

**Files to Create/Modify:**
```
crates/common/src/config.rs   (create)
crates/common/src/lib.rs      (modify: pub mod config)
config/default.toml           (create — dev defaults)
```

**Method Signatures:**
```rust
// crates/common/src/config.rs

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub vault: VaultConfig,
    pub database: DatabaseConfig,
    pub scheduler: SchedulerConfig,
    pub alerts: AlertsConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct VaultConfig {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct SchedulerConfig {
    pub default_interval_minutes: u32,
}

#[derive(Debug, Deserialize)]
pub struct AlertsConfig {
    pub drift_threshold_kl: f32,
    pub drift_threshold_cos: f32,
}

impl AppConfig {
    /// Load from a TOML file path.
    pub fn load(path: &Path) -> Result<Self>;

    /// Validate all fields after deserializing.
    pub fn validate(&self) -> Result<()>;
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn config_loads_from_default_toml();          // reads config/default.toml included as test fixture
    #[test]
    fn config_validate_rejects_port_zero();
    #[test]
    fn config_validate_rejects_negative_threshold();
    #[test]
    fn missing_required_field_returns_config_error();
}
```

**Definition of Done:**
- [ ] `AppConfig::load("config/default.toml")` returns `Ok`
- [ ] Invalid config returns structured `ModelSentryError::Config`
- [ ] All tests pass

---

## Phase 2 — Drift Algorithms

### Task 2.1 — Embedding Utilities

**Description:** Newtype `Embedding` wrapping `Vec<f32>` with checked constructors. Helper functions: centroid computation, dot product, L2 norm.

**Files to Create/Modify:**
```
crates/core/src/drift/mod.rs        (create)
crates/core/src/lib.rs              (modify: pub mod drift)
```

**Method Signatures:**
```rust
// crates/core/src/drift/mod.rs

/// Validated embedding vector. Constructor checks non-empty and finite values.
#[derive(Debug, Clone)]
pub struct Embedding(Vec<f32>);

impl Embedding {
    /// Returns Err if vec is empty or contains NaN/infinity.
    pub fn new(raw: Vec<f32>) -> Result<Self>;
    pub fn dim(&self) -> usize;
    pub fn as_slice(&self) -> &[f32];

    /// Pairwise arithmetic mean of a slice of embeddings. All must share same dim.
    pub fn centroid(embeddings: &[Self]) -> Result<Self>;

    pub fn dot(&self, other: &Self) -> Result<f32>;
    pub fn l2_norm(&self) -> f32;
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn embedding_rejects_empty_vec();
    #[test]
    fn embedding_rejects_nan();
    #[test]
    fn embedding_rejects_infinity();
    #[test]
    fn centroid_of_one_is_itself();
    #[test]
    fn centroid_of_two_is_midpoint();
    #[test]
    fn centroid_rejects_mismatched_dims();
    #[test]
    fn l2_norm_of_unit_vector_is_one();

    // proptest
    proptest! {
        #[test]
        fn centroid_dim_equals_input_dim(vecs: Vec<Vec<f32>>) { ... }
        #[test]
        fn l2_norm_is_nonnegative(v: Vec<f32>) { ... }
    }
}
```

**Definition of Done:**
- [ ] `Embedding::new` rejects all invalid inputs without panicking
- [ ] All property tests pass with default proptest config (256 cases)
- [ ] `clippy::arithmetic_side_effects` lint satisfied (checked ops or explicit `allow`)

---

### Task 2.2 — KL Divergence

**Description:** Implement `kl_divergence` for two discrete probability distributions (used on output token frequency distributions). Implement `gaussian_kl` for two univariate Gaussians (used on embedding variance).

**Files to Create/Modify:**
```
crates/core/src/drift/kl.rs    (create)
crates/core/src/drift/mod.rs   (modify: pub mod kl)
```

**Method Signatures:**
```rust
// crates/core/src/drift/kl.rs

/// KL divergence D_KL(p || q) for discrete distributions.
/// Both slices must be valid probability distributions (sum ≈ 1.0, all values ≥ 0).
/// Returns Err if inputs are invalid or if q contains zeros where p > 0.
pub fn kl_divergence(p: &[f32], q: &[f32]) -> Result<f32>;

/// KL divergence between two univariate Gaussians N(μ₁,σ₁²) || N(μ₂,σ₂²).
/// Closed-form formula; returns Err if any sigma is non-positive.
pub fn gaussian_kl(mu1: f32, sigma1: f32, mu2: f32, sigma2: f32) -> Result<f32>;

/// Normalize a frequency count vector into a probability distribution.
/// Returns Err if all counts are zero.
pub fn normalize(counts: &[f32]) -> Result<Vec<f32>>;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn kl_same_distribution_is_zero();           // kl(p, p) == 0.0 (within epsilon)
    #[test]
    fn kl_is_nonnegative();
    #[test]
    fn kl_rejects_mismatched_lengths();
    #[test]
    fn kl_rejects_empty_input();
    #[test]
    fn kl_rejects_q_zero_where_p_nonzero();
    #[test]
    fn gaussian_kl_same_params_is_zero();
    #[test]
    fn gaussian_kl_rejects_zero_sigma();
    #[test]
    fn normalize_sums_to_one();
    #[test]
    fn normalize_rejects_all_zero_input();

    proptest! {
        #[test]
        fn kl_divergence_is_always_nonnegative(p: Vec<f32>, q: Vec<f32>) {
            // if both are valid distributions, result >= 0
        }
        #[test]
        fn kl_with_uniform_q_is_bounded(p: Vec<f32>) { ... }
    }
}
```

**Definition of Done:**
- [ ] All mathematical invariants verified by property tests
- [ ] No `unwrap()` or `panic!` anywhere in implementation
- [ ] `cargo bench` runs without error (bench stubs can be minimal at this stage)

---

### Task 2.3 — Cosine Distance

**Description:** Implement cosine similarity and cosine distance between two `Embedding` values.

**Files to Create/Modify:**
```
crates/core/src/drift/cosine.rs   (create)
crates/core/src/drift/mod.rs      (modify: pub mod cosine)
```

**Method Signatures:**
```rust
// crates/core/src/drift/cosine.rs

/// Cosine similarity in [-1, 1]. Returns Err for mismatched dimensions or zero-norm vectors.
pub fn cosine_similarity(a: &Embedding, b: &Embedding) -> Result<f32>;

/// Cosine distance in [0, 1] = (1 - cosine_similarity) / 2.
/// Normalizes similarity range to a true distance in unit interval.
pub fn cosine_distance(a: &Embedding, b: &Embedding) -> Result<f32>;

/// Cosine distance from a point to a set centroid.
/// Equivalent to cosine_distance(embedding, &Embedding::centroid(set)).
pub fn distance_to_centroid(embedding: &Embedding, set: &[Embedding]) -> Result<f32>;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn cosine_same_vector_is_zero_distance();
    #[test]
    fn cosine_orthogonal_vectors_is_half_distance();   // [1,0] and [0,1] → distance = 0.5
    #[test]
    fn cosine_opposite_vectors_is_one_distance();
    #[test]
    fn cosine_rejects_zero_norm_vector();
    #[test]
    fn cosine_rejects_mismatched_dims();
    #[test]
    fn distance_to_centroid_of_identical_set_is_zero();

    proptest! {
        #[test]
        fn cosine_distance_is_in_unit_interval(a: Vec<f32>, b: Vec<f32>) { ... }
        #[test]
        fn cosine_distance_is_symmetric(a: Vec<f32>, b: Vec<f32>) { ... }
    }
}
```

**Definition of Done:**
- [ ] `cosine_distance(v, v) == 0.0` confirmed by test
- [ ] Range `[0.0, 1.0]` confirmed by property test
- [ ] Symmetry `d(a,b) == d(b,a)` confirmed by property test

---

### Task 2.4 — Output Entropy

**Description:** Compute per-prompt output entropy from token frequency distributions. Compute delta entropy between a run and a baseline.

**Files to Create/Modify:**
```
crates/core/src/drift/entropy.rs   (create)
crates/core/src/drift/mod.rs       (modify: pub mod entropy)
```

**Method Signatures:**
```rust
// crates/core/src/drift/entropy.rs

/// Shannon entropy H(X) = -Σ p(x) log₂ p(x) for a token frequency map.
/// Input: list of tokens from one completion. Returns entropy in bits.
pub fn token_entropy(tokens: &[String]) -> f32;

/// Mean entropy across a list of completions.
pub fn mean_entropy(completions: &[Vec<String>]) -> Result<f32>;

/// Delta entropy: |H(run) - H(baseline)|.
pub fn entropy_delta(
    run_completions: &[Vec<String>],
    baseline_completions: &[Vec<String>],
) -> Result<f32>;

/// Simple whitespace tokenizer. Split on whitespace, lowercase, strip punctuation.
pub fn tokenize(text: &str) -> Vec<String>;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn entropy_of_single_token_is_zero();
    #[test]
    fn entropy_of_uniform_two_tokens_is_one_bit();
    #[test]
    fn entropy_is_nonnegative();
    #[test]
    fn entropy_delta_same_completions_is_zero();
    #[test]
    fn tokenize_lowercases_and_strips_punctuation();
    #[test]
    fn mean_entropy_rejects_empty_input();
}
```

**Definition of Done:**
- [ ] `token_entropy(["a"])` == 0.0 (verified in test)
- [ ] All tests pass
- [ ] `entropy_delta` handles empty completion lists gracefully (returns `Err`, not panic)

---

### Task 2.5 — Drift Calculator (composition)

**Description:** Compose KL, cosine, and entropy into a single `DriftCalculator` that takes a `ProbeRun` + `BaselineSnapshot` and produces a `DriftReport`.

**Files to Create/Modify:**
```
crates/core/src/drift/calculator.rs   (create)
crates/core/src/drift/mod.rs          (modify: pub mod calculator; pub use)
```

**Method Signatures:**
```rust
// crates/core/src/drift/calculator.rs

pub struct DriftCalculator {
    kl_threshold: f32,
    cosine_threshold: f32,
}

impl DriftCalculator {
    pub fn new(kl_threshold: f32, cosine_threshold: f32) -> Result<Self>;

    /// Compute a full DriftReport given a run and its baseline.
    pub fn compute(
        &self,
        run: &ProbeRun,
        baseline: &BaselineSnapshot,
    ) -> Result<DriftReport>;

    /// Map raw metric values to a DriftLevel using configured thresholds.
    fn classify_level(&self, kl: f32, cosine: f32) -> DriftLevel;
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    fn make_run_matching_baseline() -> (ProbeRun, BaselineSnapshot);   // test fixture helper
    fn make_run_with_drift(kl_factor: f32) -> (ProbeRun, BaselineSnapshot);

    #[test]
    fn identical_run_and_baseline_produces_none_drift();
    #[test]
    fn high_kl_produces_high_or_critical_drift();
    #[test]
    fn dimension_mismatch_returns_error();
    #[test]
    fn drift_level_thresholds_match_constructor_config();
    #[test]
    fn report_contains_correct_run_and_baseline_ids();
}
```

**Definition of Done:**
- [ ] No direct calls to `kl_divergence`/`cosine_distance` outside of `DriftCalculator` 
- [ ] `compute` on identical run/baseline returns `DriftLevel::None`
- [ ] All tests pass

---

## Phase 3 — Provider Trait & Adapters

### Task 3.1 — LlmProvider Trait

**Description:** Define the core `LlmProvider` async trait that all provider adapters implement.

**Files to Create/Modify:**
```
crates/core/src/provider/mod.rs    (create)
crates/core/src/lib.rs             (modify: pub mod provider)
```

**Method Signatures:**
```rust
// crates/core/src/provider/mod.rs

#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync + 'static {
    /// Embed a batch of texts. Returns one vector per input text.
    /// All returned vectors must have the same dimension.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>>;

    /// Complete a single prompt. Returns the raw model output string.
    async fn complete(&self, prompt: &str) -> Result<String>;

    /// Human-readable provider name for logging and error messages.
    fn provider_name(&self) -> &'static str;

    /// The embedding dimension this provider returns. Used for baseline validation.
    fn embedding_dim(&self) -> usize;
}

/// A boxed, type-erased provider. Used as the concrete type in ProbeRunner.
pub type DynProvider = Arc<dyn LlmProvider>;
```

**Test Signatures:**
```rust
// No unit tests on the trait itself.
// MockLlmProvider is defined here for use by all other test modules:

#[cfg(test)]
pub mod mock {
    use mockall::mock;

    mock! {
        pub LlmProvider {}
        #[async_trait::async_trait]
        impl LlmProvider for LlmProvider {
            async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>>;
            async fn complete(&self, prompt: &str) -> Result<String>;
            fn provider_name(&self) -> &'static str;
            fn embedding_dim(&self) -> usize;
        }
    }
}
```

**Definition of Done:**
- [ ] Trait compiles with `Send + Sync + 'static` bounds
- [ ] `MockLlmProvider` generates without errors via `mockall`
- [ ] `DynProvider` alias works: `let _: DynProvider = Arc::new(mock);`

---

### Task 3.2 — OpenAI Adapter

**Description:** Implement `LlmProvider` for OpenAI using the `/v1/embeddings` and `/v1/chat/completions` endpoints.

**Files to Create/Modify:**
```
crates/core/src/provider/openai.rs   (create)
crates/core/src/provider/mod.rs      (modify: pub mod openai)
```

**Method Signatures:**
```rust
// crates/core/src/provider/openai.rs

pub struct OpenAiProvider {
    api_key: ApiKey,
    client: reqwest::Client,   // pre-configured with timeout + TLS
    embed_model: String,       // e.g. "text-embedding-3-small"
    complete_model: String,    // e.g. "gpt-4o"
    base_url: String,          // override for Azure / proxies
}

impl OpenAiProvider {
    pub fn new(
        api_key: ApiKey,
        embed_model: impl Into<String>,
        complete_model: impl Into<String>,
    ) -> Result<Self>;

    /// Override base URL (useful for testing with a mock server).
    pub fn with_base_url(self, base_url: String) -> Self;
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>>;
    async fn complete(&self, prompt: &str) -> Result<String>;
    fn provider_name(&self) -> &'static str { "openai" }
    fn embedding_dim(&self) -> usize;  // 1536 for text-embedding-3-small
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    // Use wiremock or httpmock to avoid real API calls

    #[tokio::test]
    async fn embed_parses_openai_response_format();       // mock server returns fixture JSON
    #[tokio::test]
    async fn embed_returns_error_on_4xx();                // mock returns 401
    #[tokio::test]
    async fn embed_returns_error_on_empty_response();
    #[tokio::test]
    async fn complete_extracts_message_content();
    #[tokio::test]
    async fn provider_name_is_openai();
    #[tokio::test]
    async fn new_rejects_empty_model_string();
}
```

**Definition of Done:**
- [ ] No real HTTP calls in tests (all via mock server)
- [ ] API key is never logged (verified by checking that `Debug` output of provider does not contain test key value)
- [ ] Timeout is set on the `reqwest::Client` (30s default)
- [ ] Returns `ModelSentryError::ProviderHttp` with status code on non-200

---

### Task 3.3 — Anthropic Adapter

**Description:** Implement `LlmProvider` for Anthropic using `/v1/messages`. Note: Anthropic does not have a native embeddings endpoint — this adapter uses the completion API only; embedding is provided via a third-party embedding service or by skipping embedding for Anthropic probes.

**Files to Create/Modify:**
```
crates/core/src/provider/anthropic.rs   (create)
crates/core/src/provider/mod.rs         (modify: pub mod anthropic)
```

**Method Signatures:**
```rust
// crates/core/src/provider/anthropic.rs

pub struct AnthropicProvider {
    api_key: ApiKey,
    client: reqwest::Client,
    model: String,          // e.g. "claude-sonnet-4-5"
}

impl AnthropicProvider {
    pub fn new(api_key: ApiKey, model: impl Into<String>) -> Result<Self>;
    pub fn with_base_url(self, base_url: String) -> Self;
}

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    /// Anthropic has no native embedding API.
    /// Returns ModelSentryError::Provider with message "embeddings not supported by Anthropic provider".
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Embedding>>;

    async fn complete(&self, prompt: &str) -> Result<String>;
    fn provider_name(&self) -> &'static str { "anthropic" }
    fn embedding_dim(&self) -> usize { 0 }   // 0 signals: no embedding support
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn complete_parses_anthropic_response();
    #[tokio::test]
    async fn embed_returns_not_supported_error();
    #[tokio::test]
    async fn complete_returns_error_on_overloaded_529();
    #[tokio::test]
    async fn anthropic_version_header_is_set();   // API requires anthropic-version header
}
```

**Definition of Done:**
- [ ] `embed` returns a structured `Err` (not panic) with a clear message
- [ ] `anthropic-version` header set to `"2023-06-01"` (current stable)
- [ ] All tests pass

---

### Task 3.4 — Ollama Adapter

**Description:** Implement `LlmProvider` for Ollama (local). Ollama supports both `/api/embeddings` and `/api/generate` endpoints.

**Files to Create/Modify:**
```
crates/core/src/provider/ollama.rs   (create)
crates/core/src/provider/mod.rs      (modify: pub mod ollama)
```

**Method Signatures:**
```rust
// crates/core/src/provider/ollama.rs

pub struct OllamaProvider {
    base_url: String,    // default: "http://localhost:11434"
    client: reqwest::Client,
    embed_model: String,
    complete_model: String,
    embed_dim: usize,    // must be provided by caller; Ollama doesn't report it in API
}

impl OllamaProvider {
    pub fn new(
        base_url: String,
        embed_model: impl Into<String>,
        complete_model: impl Into<String>,
        embed_dim: usize,
    ) -> Result<Self>;
}

#[async_trait::async_trait]
impl LlmProvider for OllamaProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>>;
    async fn complete(&self, prompt: &str) -> Result<String>;
    fn provider_name(&self) -> &'static str { "ollama" }
    fn embedding_dim(&self) -> usize;
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn embed_parses_ollama_embedding_response();
    #[tokio::test]
    async fn complete_parses_ollama_generate_response();
    #[tokio::test]
    async fn connection_refused_returns_provider_error();
    #[tokio::test]
    async fn new_rejects_zero_embed_dim();
}
```

**Definition of Done:**
- [ ] Works against Ollama API format (no API key required)
- [ ] Connection refused returns `ModelSentryError::Provider`, not a panic
- [ ] All tests pass with mock server

---

## Phase 4 — Probe Runner & Scheduler

### Task 4.1 — ProbeRunner

**Description:** `ProbeRunner` orchestrates one full probe run: call the provider with all prompts, collect embeddings + completions, package into a `ProbeRun`.

**Files to Create/Modify:**
```
crates/core/src/probe_runner.rs   (create)
crates/core/src/lib.rs            (modify: pub mod probe_runner)
```

**Method Signatures:**
```rust
// crates/core/src/probe_runner.rs

pub struct ProbeRunner {
    provider: DynProvider,
}

impl ProbeRunner {
    pub fn new(provider: DynProvider) -> Self;

    /// Execute all prompts in the probe concurrently (bounded by semaphore).
    /// Returns a ProbeRun with embeddings and completions filled in.
    /// drift_report is None at this point; computed separately.
    pub async fn run(
        &self,
        probe: &Probe,
        concurrency: usize,
    ) -> Result<ProbeRun>;

    /// Run only completions (no embeddings). Used for providers without embedding support.
    pub async fn run_completions_only(
        &self,
        probe: &Probe,
        concurrency: usize,
    ) -> Result<ProbeRun>;
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    fn make_test_probe(n_prompts: usize) -> Probe;

    #[tokio::test]
    async fn run_returns_one_embedding_per_prompt();
    #[tokio::test]
    async fn run_returns_one_completion_per_prompt();
    #[tokio::test]
    async fn run_failed_embed_propagates_error();
    #[tokio::test]
    async fn run_respects_concurrency_limit();    // use mock with sleep to verify
    #[tokio::test]
    async fn run_id_is_unique_across_two_runs();
}
```

**Definition of Done:**
- [ ] Provider calls are concurrent but bounded by `concurrency` semaphore
- [ ] Partial failures (one prompt's embed fails) propagate as `RunStatus::PartialFailure`
- [ ] `ProbeRun.embeddings.len() == probe.prompts.len()` guaranteed or `Err` returned

---

### Task 4.2 — Persistence Layer (crates/store)

**Description:** Implement all four store modules using `redb`. Each module opens named tables and provides typed CRUD operations.

**Files to Create/Modify:**
```
crates/store/src/db.rs               (create)
crates/store/src/probe_store.rs      (create)
crates/store/src/baseline_store.rs   (create)
crates/store/src/run_store.rs        (create)
crates/store/src/alert_store.rs      (create)
crates/store/src/lib.rs              (modify: pub use all stores)
```

**Method Signatures:**
```rust
// crates/store/src/db.rs
pub fn open_db(path: &Path) -> Result<Database>;

// crates/store/src/probe_store.rs
pub struct ProbeStore<'db> { /* redb WriteTransaction or ReadTransaction */ }
impl<'db> ProbeStore<'db> {
    pub fn insert(&self, probe: &Probe) -> Result<()>;
    pub fn get(&self, id: &ProbeId) -> Result<Option<Probe>>;
    pub fn list_all(&self) -> Result<Vec<Probe>>;
    pub fn delete(&self, id: &ProbeId) -> Result<bool>;   // returns false if not found
    pub fn update(&self, probe: &Probe) -> Result<()>;
}

// crates/store/src/baseline_store.rs
pub struct BaselineStore<'db> { ... }
impl<'db> BaselineStore<'db> {
    pub fn insert(&self, baseline: &BaselineSnapshot) -> Result<()>;
    pub fn get_latest_for_probe(&self, probe_id: &ProbeId) -> Result<Option<BaselineSnapshot>>;
    pub fn list_for_probe(&self, probe_id: &ProbeId) -> Result<Vec<BaselineSnapshot>>;
    pub fn delete(&self, id: &BaselineId) -> Result<bool>;
}

// crates/store/src/run_store.rs
pub struct RunStore<'db> { ... }
impl<'db> RunStore<'db> {
    pub fn insert(&self, run: &ProbeRun) -> Result<()>;
    pub fn get(&self, id: &RunId) -> Result<Option<ProbeRun>>;
    pub fn list_for_probe(
        &self,
        probe_id: &ProbeId,
        limit: usize,
    ) -> Result<Vec<ProbeRun>>;
}

// crates/store/src/alert_store.rs
pub struct AlertRuleStore<'db> { ... }
impl<'db> AlertRuleStore<'db> {
    pub fn insert_rule(&self, rule: &AlertRule) -> Result<()>;
    pub fn get_rules_for_probe(&self, probe_id: &ProbeId) -> Result<Vec<AlertRule>>;
    pub fn insert_event(&self, event: &AlertEvent) -> Result<()>;
    pub fn list_events(&self, limit: usize) -> Result<Vec<AlertEvent>>;
    pub fn acknowledge_event(&self, id: &Uuid) -> Result<bool>;
}
```

**Test Signatures:**
```rust
// Each store module has a test module using tempfile::tempdir()
#[cfg(test)]
mod tests {
    fn open_test_db() -> (TempDir, Database);   // fixture helper

    // probe_store tests:
    #[test]
    fn insert_and_get_probe();
    #[test]
    fn get_nonexistent_probe_returns_none();
    #[test]
    fn delete_probe_returns_false_if_not_found();
    #[test]
    fn list_all_returns_all_inserted_probes();

    // baseline_store tests:
    #[test]
    fn get_latest_returns_most_recent_baseline();
    #[test]
    fn multiple_baselines_for_same_probe_stored_correctly();

    // run_store tests:
    #[test]
    fn list_for_probe_respects_limit();
    #[test]
    fn list_for_probe_ordered_newest_first();

    // alert_store tests:
    #[test]
    fn acknowledge_event_sets_acknowledged_true();
    #[test]
    fn acknowledge_nonexistent_event_returns_false();
}
```

**Definition of Done:**
- [ ] All tests pass with a fresh `tempdir` per test (no shared state)
- [ ] Store functions never return `redb::Error` directly — always wrapped in `ModelSentryError::Storage`
- [ ] No `unwrap()` in implementation

---

### Task 4.3 — Scheduler

**Description:** Tokio-based scheduler that loads all probes from the store, sets up a timer per probe according to its `ProbeSchedule`, and runs `ProbeRunner` on tick. Writes results back to store.

**Files to Create/Modify:**
```
crates/daemon/src/scheduler.rs   (create)
```

**Method Signatures:**
```rust
// crates/daemon/src/scheduler.rs

pub struct Scheduler {
    store: Arc<AppStore>,        // combined handle wrapping all store types
    providers: ProviderRegistry, // map from ProviderKind → DynProvider
    calculator: DriftCalculator,
    alert_engine: AlertEngine,
}

impl Scheduler {
    pub fn new(
        store: Arc<AppStore>,
        providers: ProviderRegistry,
        calculator: DriftCalculator,
        alert_engine: AlertEngine,
    ) -> Self;

    /// Start the scheduler. Returns a handle; call `.shutdown()` to stop.
    pub async fn start(self) -> Result<SchedulerHandle>;
}

pub struct SchedulerHandle {
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl SchedulerHandle {
    pub async fn shutdown(self);
}

// Internal (not pub):
async fn run_probe_job(
    probe: Probe,
    store: Arc<AppStore>,
    provider: DynProvider,
    calculator: &DriftCalculator,
    alert_engine: &AlertEngine,
);
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn scheduler_runs_probe_on_tick();          // mock provider, advance time
    #[tokio::test]
    async fn scheduler_writes_run_to_store();
    #[tokio::test]
    async fn scheduler_shuts_down_cleanly();
    #[tokio::test]
    async fn probe_run_failure_does_not_crash_scheduler();
}
```

**Definition of Done:**
- [ ] A probe run failure logs at `ERROR` level but does not crash the scheduler loop
- [ ] `SchedulerHandle::shutdown()` joins all tasks without timeout panic
- [ ] Test uses `tokio::time::pause()` + `tokio::time::advance()` for deterministic scheduling

---

## Phase 5 — Alert Engine & Vault

### Task 5.1 — Alert Engine

**Description:** Evaluate a `DriftReport` against all `AlertRule` records for a probe. Fire notifications for any triggered rules.

**Files to Create/Modify:**
```
crates/core/src/alert.rs    (create)
crates/core/src/lib.rs      (modify: pub mod alert)
```

**Method Signatures:**
```rust
// crates/core/src/alert.rs

pub struct AlertEngine {
    http_client: reqwest::Client,
}

impl AlertEngine {
    pub fn new(http_client: reqwest::Client) -> Self;

    /// Check report against all rules. Fire any rules that are triggered.
    /// Returns all AlertEvents created (empty if no rules triggered).
    pub async fn evaluate_and_fire(
        &self,
        report: &DriftReport,
        rules: &[AlertRule],
    ) -> Vec<AlertEvent>;

    fn is_triggered(report: &DriftReport, rule: &AlertRule) -> bool;

    async fn fire_channel(
        &self,
        channel: &AlertChannel,
        event: &AlertEvent,
    ) -> Result<()>;
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    fn make_report_with_kl(kl: f32) -> DriftReport;
    fn make_rule_with_threshold(kl: f32) -> AlertRule;

    #[tokio::test]
    async fn report_below_threshold_fires_no_alert();
    #[tokio::test]
    async fn report_above_kl_threshold_fires_alert();
    #[tokio::test]
    async fn report_above_cosine_threshold_fires_alert();
    #[tokio::test]
    async fn webhook_channel_posts_correct_json();    // wiremock mock server
    #[tokio::test]
    async fn failed_webhook_does_not_panic();         // mock returns 500
    #[tokio::test]
    async fn multiple_rules_each_produce_own_event();
}
```

**Definition of Done:**
- [ ] A failed webhook delivery logs at `WARN` level and returns empty `AlertEvent` without propagating error to caller
- [ ] `is_triggered` is a pure function (no I/O) — fully unit tested
- [ ] All tests pass

---

### Task 5.2 — API Key Vault

**Description:** Encrypt and decrypt API keys using `age`. Keys are stored in an `age`-encrypted file. At runtime, keys are held in `Secret<String>` and never written to disk unencrypted.

**Files to Create/Modify:**
```
crates/daemon/src/vault.rs   (create)
```

**Method Signatures:**
```rust
// crates/daemon/src/vault.rs

/// A map of provider identifiers to their encrypted API keys.
pub struct Vault {
    path: PathBuf,
    passphrase: Secret<String>,
}

impl Vault {
    pub fn open(path: &Path, passphrase: Secret<String>) -> Result<Self>;
    pub fn create(path: &Path, passphrase: Secret<String>) -> Result<Self>;

    pub fn get_key(&self, provider_id: &str) -> Result<Option<ApiKey>>;
    pub fn set_key(&self, provider_id: &str, key: ApiKey) -> Result<()>;
    pub fn delete_key(&self, provider_id: &str) -> Result<bool>;
    pub fn list_providers(&self) -> Result<Vec<String>>;
}

// Internal (not pub):
fn decrypt_vault(path: &Path, passphrase: &Secret<String>) -> Result<BTreeMap<String, String>>;
fn encrypt_and_write(
    path: &Path,
    passphrase: &Secret<String>,
    data: &BTreeMap<String, String>,
) -> Result<()>;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn create_and_reopen_vault_retrieves_key();
    #[test]
    fn wrong_passphrase_returns_vault_error();
    #[test]
    fn vault_file_is_opaque_binary_not_plaintext();  // assert file bytes don't contain key string
    #[test]
    fn delete_key_returns_false_when_absent();
    #[test]
    fn set_key_overwrites_existing_key();
}
```

**Definition of Done:**
- [ ] Vault file bytes assert not to contain the raw key string
- [ ] `get_key` never logs the key value (confirmed by tracing subscriber in test)
- [ ] All tests use `tempdir` for file isolation

---

## Phase 6 — REST API

### Task 6.1 — Axum App State & Router

**Description:** Wire the axum router with shared `AppState`. Define all route prefixes. No handler logic yet — each route returns `501 Not Implemented`.

**Files to Create/Modify:**
```
crates/daemon/src/server.rs        (create)
crates/daemon/src/routes/mod.rs    (create)
crates/daemon/src/main.rs          (modify: call server::run)
```

**Method Signatures:**
```rust
// crates/daemon/src/server.rs

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<AppStore>,
    pub scheduler: Arc<Scheduler>,
    pub vault: Arc<Vault>,
    pub calculator: Arc<DriftCalculator>,
    pub alert_engine: Arc<AlertEngine>,
    pub config: Arc<AppConfig>,
}

pub fn build_router(state: AppState) -> axum::Router;

pub async fn run(config: &AppConfig, state: AppState) -> Result<()>;
```

**Definition of Done:**
- [ ] `cargo run -p modelsentry-daemon` starts without error
- [ ] `curl http://localhost:7740/api/probes` returns HTTP 501
- [ ] Tower middleware: `TraceLayer`, `TimeoutLayer(30s)`, `CorsLayer` applied

---

### Task 6.2 — Probes API

**Description:** Implement all CRUD endpoints for probes.

**Files to Create/Modify:**
```
crates/daemon/src/routes/probes.rs   (create)
```

**Method Signatures (route handlers):**
```rust
// GET /api/probes
async fn list_probes(State(state): State<AppState>)
    -> Result<Json<Vec<Probe>>, AppError>;

// POST /api/probes
async fn create_probe(
    State(state): State<AppState>,
    Json(body): Json<CreateProbeRequest>,
) -> Result<(StatusCode, Json<Probe>), AppError>;

// GET /api/probes/:id
async fn get_probe(
    State(state): State<AppState>,
    Path(id): Path<ProbeId>,
) -> Result<Json<Probe>, AppError>;

// DELETE /api/probes/:id
async fn delete_probe(
    State(state): State<AppState>,
    Path(id): Path<ProbeId>,
) -> Result<StatusCode, AppError>;

// POST /api/probes/:id/run-now
async fn trigger_probe_run(
    State(state): State<AppState>,
    Path(id): Path<ProbeId>,
) -> Result<Json<ProbeRun>, AppError>;

#[derive(Deserialize)]
pub struct CreateProbeRequest {
    pub name: String,
    pub provider: ProviderKind,
    pub model: String,
    pub prompts: Vec<ProbePrompt>,
    pub schedule: ProbeSchedule,
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    fn test_app() -> axum::Router;   // builds app with in-memory store + mock provider

    #[tokio::test]
    async fn list_probes_returns_empty_initially();
    #[tokio::test]
    async fn create_probe_returns_201_with_probe_body();
    #[tokio::test]
    async fn get_probe_returns_404_for_unknown_id();
    #[tokio::test]
    async fn delete_probe_returns_204();
    #[tokio::test]
    async fn create_probe_with_missing_name_returns_422();
    #[tokio::test]
    async fn run_now_returns_probe_run();

    // insta snapshot tests:
    #[tokio::test]
    async fn create_probe_response_shape();  // insta::assert_json_snapshot!
}
```

**Definition of Done:**
- [ ] All routes return correct HTTP status codes per REST conventions
- [ ] `AppError` converts `ModelSentryError::ProbeNotFound` to `404 Not Found`
- [ ] Insta snapshots committed
- [ ] All tests pass

---

### Task 6.3 — Baselines, Runs, Alerts APIs

**Description:** Implement route handlers for baselines, runs, and alert rules/events.

**Files to Create/Modify:**
```
crates/daemon/src/routes/baselines.rs   (create)
crates/daemon/src/routes/runs.rs        (create)
crates/daemon/src/routes/alerts.rs      (create)
```

**Route Signatures:**
```
GET  /api/probes/:id/baselines           → Vec<BaselineSnapshot>
POST /api/probes/:id/baselines           → BaselineSnapshot (capture now from last run)
GET  /api/probes/:id/baselines/latest    → BaselineSnapshot
DELETE /api/baselines/:id                → 204

GET  /api/probes/:id/runs                → Vec<ProbeRun> (paginated, ?limit=20)
GET  /api/runs/:id                       → ProbeRun

GET  /api/probes/:id/alerts              → Vec<AlertRule>
POST /api/probes/:id/alerts              → AlertRule
DELETE /api/alerts/:id                   → 204
GET  /api/events                         → Vec<AlertEvent> (?limit=50)
POST /api/events/:id/acknowledge         → 204
```

**Test Signatures (one representative test per route group):**
```rust
#[tokio::test]
async fn capture_baseline_requires_at_least_one_run();
#[tokio::test]
async fn list_runs_respects_limit_query_param();
#[tokio::test]
async fn create_alert_rule_with_webhook_channel();
#[tokio::test]
async fn acknowledge_event_returns_404_for_unknown();
```

**Definition of Done:**
- [ ] All routes registered in `build_router`
- [ ] `GET /api/probes/:id/runs?limit=5` returns at most 5 runs
- [ ] All handler tests pass

---

## Phase 7 — Svelte Dashboard

### Task 7.1 — API Client & Types

**Description:** Typed TypeScript wrappers for all REST endpoints. Mirror Rust models exactly.

**Files to Create/Modify:**
```
web/src/lib/types.ts   (create)
web/src/lib/api.ts     (create)
```

**Type Signatures (TypeScript):**
```ts
// web/src/lib/types.ts
export interface ProbeId { value: string }        // opaque wrappers for safety
export interface Probe {
  id: string;
  name: string;
  provider: ProviderKind;
  model: string;
  prompts: ProbePrompt[];
  schedule: ProbeSchedule;
  created_at: string;
}
export interface DriftReport {
  run_id: string;
  baseline_id: string;
  kl_divergence: number;
  cosine_distance: number;
  output_entropy_delta: number;
  drift_level: 'none' | 'low' | 'medium' | 'high' | 'critical';
  computed_at: string;
}
// ... all models mirrored

// web/src/lib/api.ts
export const api = {
  probes: {
    list: () => Promise<Probe[]>,
    create: (body: CreateProbeRequest) => Promise<Probe>,
    get: (id: string) => Promise<Probe>,
    delete: (id: string) => Promise<void>,
    runNow: (id: string) => Promise<ProbeRun>,
  },
  baselines: {
    listForProbe: (probeId: string) => Promise<BaselineSnapshot[]>,
    captureForProbe: (probeId: string) => Promise<BaselineSnapshot>,
  },
  runs: {
    listForProbe: (probeId: string, limit?: number) => Promise<ProbeRun[]>,
  },
  alerts: {
    listEvents: (limit?: number) => Promise<AlertEvent[]>,
    acknowledgeEvent: (id: string) => Promise<void>,
  },
};
```

**Definition of Done:**
- [ ] `npm run build` passes with zero TypeScript errors
- [ ] All API functions use `zod` for response validation (schema matches Rust model)
- [ ] Errors from non-2xx responses throw a typed `ApiError` class

---

### Task 7.2 — Dashboard Overview Page

**Description:** The `/` route shows: summary cards (total probes, last run status, active drift alerts), a drift score timeline chart per probe for the last 7 days.

**Files to Create/Modify:**
```
web/src/routes/+page.svelte           (modify from stub)
web/src/lib/components/DriftChart.svelte    (create)
web/src/lib/components/SummaryCard.svelte   (create)
```

**Component Props:**
```ts
// DriftChart.svelte
export let probeId: string;
export let runs: ProbeRun[];       // pre-fetched by parent
export let baseline: BaselineSnapshot | null;

// SummaryCard.svelte
export let title: string;
export let value: string | number;
export let status: 'ok' | 'warn' | 'error' | 'neutral';
```

**Definition of Done:**
- [ ] Chart renders KL divergence over time using Chart.js line chart
- [ ] Red horizontal line shows configured threshold on the chart
- [ ] `DriftLevel.critical` or `high` causes card background to turn red
- [ ] Page loads in <500ms on localhost (no waterfalls — parallel `Promise.all` fetch)

---

### Task 7.3 — Probes Management Page

**Description:** `/probes` — table of all probes with status badges. `/probes/[id]` — detail view: prompt list, last run, current drift metrics, run-now button.

**Files to Create/Modify:**
```
web/src/routes/probes/+page.svelte            (create)
web/src/routes/probes/[id]/+page.svelte       (create)
web/src/lib/components/ProbeTable.svelte      (create)
web/src/lib/components/DriftMetrics.svelte    (create)
```

**Definition of Done:**
- [ ] Table sortable by name and last drift level
- [ ] "Run Now" button triggers `POST /api/probes/:id/run-now` and shows result toast
- [ ] Alert feed sidebar visible on detail page showing last 5 alert events

---

## Phase 8 — CLI

### Task 8.1 — CLI Entry Point & Probe Commands

**Description:** `modelsentry-cli` binary with clap derive macros. Communicates with the daemon's REST API.

**Files to Create/Modify:**
```
crates/cli/src/main.rs             (modify)
crates/cli/src/commands/probe.rs   (create)
```

**Method Signatures:**
```rust
// crates/cli/src/main.rs

#[derive(Parser)]
#[command(name = "modelsentry", about = "ModelSentry CLI")]
struct Cli {
    #[arg(long, default_value = "http://localhost:7740")]
    api_url: String,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Probe(ProbeArgs),
    Baseline(BaselineArgs),
    Alert(AlertArgs),
}

// crates/cli/src/commands/probe.rs

#[derive(Args)]
pub struct ProbeArgs {
    #[command(subcommand)]
    pub action: ProbeAction,
}

#[derive(Subcommand)]
pub enum ProbeAction {
    /// List all configured probes
    List,
    /// Add a new probe from a TOML config file
    Add { #[arg(long)] config: PathBuf },
    /// Delete a probe by ID
    Delete { id: String },
    /// Trigger an immediate probe run
    RunNow { id: String },
    /// Show the last drift report for a probe
    Status { id: String },
}

pub async fn handle(args: ProbeArgs, api_url: &str) -> anyhow::Result<()>;
```

**Definition of Done:**
- [ ] `modelsentry probe list` prints a table with probe name, provider, last drift level
- [ ] `modelsentry probe run-now <id>` prints drift metrics on completion
- [ ] `modelsentry probe add --config my_probe.toml` creates probe and prints the new ID
- [ ] All output goes to stdout; errors go to stderr with exit code 1

---

## Phase 9 — Integration Tests & Benchmarks

### Task 9.1 — End-to-End Probe Lifecycle Test

**Description:** Full lifecycle test: create probe → capture baseline → simulate run with drift → assert alert fired. Uses mock LLM provider server (wiremock).

**Files to Create/Modify:**
```
tests/integration/probe_lifecycle.rs   (create)
tests/integration/drift_detection.rs   (create)
tests/integration/alert_fire.rs        (create)
```

**Test Signatures:**
```rust
// tests/integration/probe_lifecycle.rs

#[tokio::test]
async fn full_lifecycle_create_probe_capture_baseline_detect_drift();

#[tokio::test]
async fn probe_survives_provider_flake_and_retries();

#[tokio::test]
async fn delete_probe_also_deletes_associated_runs_and_baselines();

// tests/integration/drift_detection.rs

#[tokio::test]
async fn no_drift_detected_when_model_stable();

#[tokio::test]
async fn drift_detected_when_embedding_centroid_shifts_beyond_threshold();

// tests/integration/alert_fire.rs

#[tokio::test]
async fn webhook_receives_correct_payload_on_drift_event();
```

**Definition of Done:**
- [ ] All integration tests pass in CI without any external service
- [ ] `cargo nextest run --workspace` completes in <30 seconds
- [ ] No shared mutable state between test functions

---

### Task 9.2 — Benchmark Suite

**Description:** `criterion` benchmarks for the three drift algorithms at representative embedding sizes.

**Files to Create/Modify:**
```
crates/core/benches/drift_bench.rs   (create)
crates/core/Cargo.toml               (modify: add [[bench]] entry)
```

**Benchmark Signatures:**
```rust
// crates/core/benches/drift_bench.rs

// bench groups:
fn bench_kl_divergence(c: &mut Criterion);    // N = 100, 500, 1536
fn bench_cosine_distance(c: &mut Criterion);  // N = 100, 500, 1536
fn bench_output_entropy(c: &mut Criterion);   // n_completions = 10, 50, 100 tokens each
fn bench_drift_calculator_compute(c: &mut Criterion);  // full DriftCalculator.compute()
```

**Definition of Done:**
- [ ] `cargo bench` runs without error
- [ ] Baseline results committed to repo for CI regression detection
- [ ] `kl_divergence` at N=1536 completes in <100µs on dev machine (criterion confirms)

---

## Phase 10 — Polish & Release Prep

### Task 10.1 — README & Quickstart

**Files to Create/Modify:**
```
modelsentry/README.md   (create)
config/default.toml     (finalize)
```

**Definition of Done:**
- [x] README covers: install → configure → first probe → first baseline → first alert
- [x] `cargo install --path crates/daemon` documented
- [x] Docker Compose file for one-command local setup

---

### Task 10.2 — Release CI

**Files to Create/Modify:**
```
.github/workflows/release.yml   (create)
```

**Definition of Done:**
- [ ] On `git tag v*`, workflow builds Linux x86-64 and macOS aarch64 binaries
- [ ] Binaries attached to GitHub Release
- [ ] Frontend static build embedded into daemon binary via `include_dir!` macro

---

## Phase 11 — Sellability and Reliability Hardening

### Task 11.1 — Provider Capability Resolver and Metric Pipeline Router

**Description:** Implement capability-aware metric routing so every probe run uses only valid metrics for the selected provider/model.

**Files to Create/Modify:**
```
crates/core/src/provider/capabilities.rs      (create)
crates/core/src/drift/pipeline.rs             (create)
crates/core/src/drift/calculator.rs           (modify)
crates/common/src/models.rs                   (modify: add metric_capability metadata)
```

**Method Signatures:**
```rust
// crates/core/src/provider/capabilities.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricCapability {
    EmbeddingAndText,
    TextOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub metric_capability: MetricCapability,
    pub supports_embeddings: bool,
    pub supports_completions: bool,
}

pub trait CapabilityResolver {
    fn capabilities_for(&self, provider: &ProviderKind, model: &str) -> ProviderCapabilities;
}

// crates/core/src/drift/pipeline.rs

pub enum MetricPipeline {
    EmbeddingMetricsPipeline,
    TextOnlyPipeline,
}

impl MetricPipeline {
    pub fn for_capability(capability: MetricCapability) -> Self;

    pub fn compute(
        &self,
        run: &ProbeRun,
        baseline: &BaselineSnapshot,
        cfg: &DriftConfig,
    ) -> Result<DriftReport>;
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn anthropic_maps_to_text_only_pipeline();
    #[test]
    fn openai_maps_to_embedding_pipeline();
    #[test]
    fn text_only_pipeline_does_not_require_embeddings();
    #[test]
    fn embedding_pipeline_errors_on_missing_embeddings();
}
```

**Definition of Done:**
- [ ] `DriftReport` includes `pipeline_used` field
- [ ] API returns capability metadata per probe
- [ ] No provider can accidentally execute unsupported metrics

---

### Task 11.2 — Alert Stability Policy (False-Positive Suppression)

**Description:** Add suppression policy layer to avoid alert spam and make confidence explicit.

**Files to Create/Modify:**
```
crates/core/src/alert_policy.rs               (create)
crates/core/src/alert.rs                      (modify)
crates/common/src/models.rs                   (modify: add stability policy and confidence fields)
config/default.toml                           (modify: add default stability policy)
```

**Method Signatures:**
```rust
// crates/core/src/alert_policy.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertStabilityPolicy {
    pub warmup_runs: u32,
    pub consecutive_breaches: u32,
    pub cooldown_minutes: u32,
    pub min_confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuppressionReason {
    Warmup,
    NotEnoughConsecutiveBreaches,
    CooldownActive,
    ConfidenceTooLow,
}

pub struct AlertDecision {
    pub should_fire: bool,
    pub confidence: f32,
    pub suppression_reason: Option<SuppressionReason>,
}

pub fn evaluate_alert_policy(
    policy: &AlertStabilityPolicy,
    context: &AlertEvaluationContext,
) -> AlertDecision;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn warmup_suppresses_alerts();
    #[test]
    fn consecutive_breach_requirement_is_enforced();
    #[test]
    fn cooldown_blocks_repeated_alerts();
    #[test]
    fn low_confidence_blocks_alert();
    #[test]
    fn decision_includes_suppression_reason_when_suppressed();
}
```

**Definition of Done:**
- [ ] Alert payload includes `confidence` and `suppression_reason`
- [ ] Dashboard can display why an alert was suppressed
- [ ] Alert volume drops in synthetic noisy test scenario without missing sustained drift

---

### Task 11.3 — Baseline Lifecycle (Candidate/Active/Retired)

**Description:** Replace implicit baseline behavior with explicit lifecycle and promotion workflow.

**Files to Create/Modify:**
```
crates/common/src/models.rs                   (modify: baseline state)
crates/store/src/baseline_store.rs            (modify)
crates/daemon/src/routes/baselines.rs         (modify)
web/src/routes/baselines/+page.svelte         (modify)
```

**Method Signatures:**
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaselineState {
    Candidate,
    Active,
    Retired,
}

impl BaselineStore<'_> {
    pub fn create_candidate(&self, baseline: &BaselineSnapshot) -> Result<()>;
    pub fn promote_to_active(&self, baseline_id: &BaselineId) -> Result<()>;
    pub fn get_active_for_probe(&self, probe_id: &ProbeId) -> Result<Option<BaselineSnapshot>>;
    pub fn retire(&self, baseline_id: &BaselineId) -> Result<bool>;
}

// routes
// POST /api/baselines/:id/promote
async fn promote_baseline(
    State(state): State<AppState>,
    Path(id): Path<BaselineId>,
) -> Result<StatusCode, AppError>;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn first_promoted_baseline_becomes_active();
    #[test]
    fn promoting_new_baseline_retires_previous_active();
    #[test]
    fn only_one_active_baseline_per_probe_is_allowed();
    #[tokio::test]
    async fn promote_endpoint_returns_204_on_success();
}
```

**Definition of Done:**
- [ ] Exactly one active baseline per probe is enforced at store layer
- [ ] UI shows candidate/active/retired badges
- [ ] Drift computations always use active baseline only

---

### Task 11.4 — Budget Controller and Adaptive Sampling

**Description:** Add budget policy and adaptive scheduling to control probe spend and improve operational adoption.

**Files to Create/Modify:**
```
crates/common/src/models.rs                   (modify: budget policy)
crates/daemon/src/scheduler.rs                (modify)
crates/core/src/probe_runner.rs               (modify)
web/src/routes/probes/[id]/+page.svelte       (modify)
config/default.toml                           (modify: budget defaults)
```

**Method Signatures:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetPolicy {
    pub monthly_token_cap: u64,
    pub stable_interval_minutes: u32,
    pub anomaly_interval_minutes: u32,
    pub hard_stop_on_budget_exhausted: bool,
}

pub struct BudgetDecision {
    pub allow_run: bool,
    pub next_interval_minutes: u32,
    pub reason: Option<String>,
}

pub fn evaluate_budget_policy(
    policy: &BudgetPolicy,
    usage: &ProbeUsageWindow,
    last_drift_level: DriftLevel,
) -> BudgetDecision;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn budget_exhausted_blocks_run_when_hard_stop_enabled();
    #[test]
    fn stable_period_uses_longer_interval();
    #[test]
    fn anomaly_period_uses_shorter_interval();
    #[test]
    fn usage_rollover_resets_monthly_budget_counter();
}
```

**Definition of Done:**
- [ ] Scheduler adjusts next run interval based on stability/anomaly state
- [ ] Probe details page shows current month budget usage and projected exhaustion date
- [ ] Runs are blocked with explicit reason when budget policy says no

---

### Task 11.5 — Drift-to-Impact Correlation (Sellability Feature)

**Description:** Add optional KPI correlation so users can tie drift to business outcomes.

**Files to Create/Modify:**
```
crates/common/src/models.rs                   (modify: KPI event model)
crates/core/src/impact.rs                     (create)
crates/daemon/src/routes/impact.rs            (create)
crates/daemon/src/routes/mod.rs               (modify)
web/src/lib/components/ImpactPanel.svelte     (create)
web/src/routes/+page.svelte                   (modify)
```

**Method Signatures:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpiEvent {
    pub id: Uuid,
    pub metric_name: String,
    pub value: f64,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftImpactCorrelation {
    pub probe_id: ProbeId,
    pub lag_minutes: i64,
    pub correlation: f64,
    pub confidence: f32,
}

pub fn correlate_drift_with_kpi(
    drift_events: &[DriftReport],
    kpi_events: &[KpiEvent],
    max_lag_minutes: i64,
) -> Result<Vec<DriftImpactCorrelation>>;
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn detects_positive_correlation_with_known_lag();
    #[test]
    fn returns_empty_when_series_too_short();
    #[test]
    fn correlation_output_is_sorted_by_abs_strength_desc();
}
```

**Definition of Done:**
- [ ] `/api/impact/correlations` returns deterministic results for fixed fixtures
- [ ] Dashboard impact panel shows strongest correlation + lag
- [ ] Feature can be disabled with config flag when KPI data is not available

---

### Task 11.6 — Enterprise Readiness Hooks (v1.5)

**Description:** Add architecture hooks for SSO/RBAC/audit export without implementing full auth stack yet.

**Files to Create/Modify:**
```
crates/daemon/src/auth/mod.rs                 (create)
crates/common/src/models.rs                   (modify: audit event model)
crates/daemon/src/server.rs                   (modify: auth middleware interface)
crates/daemon/src/routes/audit.rs             (create)
.github/workflows/release.yml                 (modify)
```

**Method Signatures:**
```rust
pub trait Authenticator: Send + Sync {
    fn authenticate(&self, headers: &axum::http::HeaderMap) -> Result<AuthContext>;
}

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub subject: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub actor: String,
    pub action: String,
    pub resource: String,
    pub occurred_at: DateTime<Utc>,
}
```

**Test Signatures:**
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn unauthenticated_request_is_rejected_when_auth_enabled();
    #[tokio::test]
    async fn audit_event_written_for_baseline_promotion();
    #[tokio::test]
    async fn audit_export_endpoint_returns_chronological_events();
}
```

**Definition of Done:**
- [ ] Auth middleware interface exists and is pluggable
- [ ] Core actions emit audit events
- [ ] Export endpoint supports JSONL and CSV

---

## Milestone Summary

| Phase | Tasks | Primary Outputs |
|---|---|---|
| 0 — Scaffold | 0.1 – 0.3 | Workspace, CI, SvelteKit skeleton |
| 1 — Common | 1.1 – 1.4 | Newtypes, errors, models, config |
| 2 — Drift Algorithms | 2.1 – 2.5 | KL, cosine, entropy, `DriftCalculator` |
| 3 — Provider Adapters | 3.1 – 3.4 | `LlmProvider` trait, OpenAI/Anthropic/Ollama |
| 4 — Runner & Store | 4.1 – 4.3 | `ProbeRunner`, `redb` store, scheduler |
| 5 — Alerts & Vault | 5.1 – 5.2 | `AlertEngine`, `age` vault |
| 6 — REST API | 6.1 – 6.3 | All axum routes with tests |
| 7 — Frontend | 7.1 – 7.3 | Dashboard, probes page, typed API client |
| 8 — CLI | 8.1 | `modelsentry` CLI binary |
| 9 — Tests + Benches | 9.1 – 9.2 | Integration tests, criterion benchmarks |
| 10 — Polish | 10.1 – 10.2 | README, Docker, release CI |
| 11 — Sellability Hardening | 11.1 – 11.6 | capability routing, suppression policy, baseline lifecycle, budget governance, KPI correlation, enterprise hooks |
