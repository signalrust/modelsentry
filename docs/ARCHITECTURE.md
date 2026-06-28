# ModelSentry — Architecture & Engineering Standards
**Stack:** Rust (workspace) · Axum · Redb · Svelte 5 · SvelteKit  
**Last updated:** June 27, 2026

---

## 1. System Overview

ModelSentry is a self-hosted daemon that fingerprints LLM API behavior by periodically sending a fixed probe corpus to a configured endpoint, embedding each model **completion**, and comparing the resulting per-prompt output-embedding clouds against a trusted baseline with a **calibrated nonparametric two-sample test**. It alerts when a run's calibrated combined p-value falls below a configured **target false-positive rate** — so the alert threshold has a precise statistical meaning instead of being a hand-tuned magic number. See [`DRIFT_DETECTION_METHODOLOGY.md`](DRIFT_DETECTION_METHODOLOGY.md) for the full theory.

```
┌──────────────────────────────────────────────────────────────────────┐
│                         ModelSentry Daemon                           │
│                                                                      │
│  ┌────────────┐   schedule   ┌─────────────┐   embed/complete        │
│  │  Scheduler │─────────────▶│ Probe Runner│──────────────────────▶  │
│  │ (cron/tick)│              │             │  LLM Provider API        │
│  └────────────┘              └──────┬──────┘  (OpenAI / Anthropic /  │
│                                     │          Ollama)                │
│                              results│                                 │
│                                     ▼                                 │
│                          ┌──────────────────┐                        │
│                          │  Drift Calculator │                        │
│                          │  (conformal +     │                        │
│                          │   MMD/energy)     │                        │
│                          └────────┬─────────┘                        │
│                                   │ DriftReport                      │
│                          ┌────────▼─────────┐                        │
│                          │  Baseline Store   │                        │
│                          │  (redb embedded)  │                        │
│                          └────────┬─────────┘                        │
│                                   │                                  │
│                          ┌────────▼─────────┐                        │
│                          │  Alert Engine     │                        │
│                          │  (webhook/slack/  │                        │
│                          │   email)          │                        │
│                          └──────────────────┘                        │
│                                                                      │
│  ┌───────────────────────────────────────────────────────────────┐   │
│  │              Axum REST API  (port 7740)                       │   │
│  │  /api/probes /api/baselines /api/runs /api/alerts /api/vault  │   │
│  └──────────────────────────────┬────────────────────────────────┘   │
│                                 │ HTTP + WebSocket                   │
└─────────────────────────────────┼────────────────────────────────────┘
                                  │
                    ┌─────────────▼──────────────┐
                    │     SvelteKit Frontend      │
                    │  (served from /web, port    │
                    │   5173 dev / static prod)   │
                    └────────────────────────────┘
```

---

## 2. Workspace Layout

```
modelsentry/
│
├── Cargo.toml                  ← workspace root; no [package]
├── rust-toolchain.toml         ← pins stable toolchain
├── .rustfmt.toml               ← formatting rules
├── .clippy.toml                ← deny list
├── Makefile                    ← dev shortcuts (check, test, run, fmt)
├── README.md
│
├── crates/
│   │
│   ├── common/                 ← shared types; no I/O, no async
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs        ← ApiKey, ModelId, ProbeId, RunId (newtypes)
│   │       ├── error.rs        ← ModelSentryError (thiserror)
│   │       ├── config.rs       ← AppConfig (serde + toml deserialization)
│   │       └── models.rs       ← Probe, Baseline, DriftReport, AlertRule (serde)
│   │
│   ├── core/                   ← pure logic; no network I/O; fully testable
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── provider/
│   │       │   ├── mod.rs      ← LlmProvider trait (async-trait)
│   │       │   ├── openai.rs
│   │       │   ├── anthropic.rs
│   │       │   ├── azure.rs    ← Azure OpenAI (api-key header, deployment URLs)
│   │       │   └── ollama.rs
│   │       ├── drift/
│   │       │   ├── mod.rs        ← Embedding newtype (validated, finite)
│   │       │   ├── twosample.rs  ← MMD² (RBF/median), energy distance, permutation test
│   │       │   ├── assessment.rs ← per-prompt conformal + stratified permutation gate + pooled fallback + severity
│   │       │   ├── calculator.rs ← DriftCalculator::compute() → DriftReport
│   │       │   └── interpret.rs  ← honest statistical-verdict interpretation
│   │       ├── probe_runner.rs ← ProbeRunner struct
│   │       ├── alert.rs        ← AlertEngine, webhook firing, cooldown + alpha-spending
│   │       └── email.rs        ← EmailMailer (SMTP, lettre)
│   │
│   ├── store/                  ← persistence layer (redb)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs          ← AppStore, open_db()
│   │       ├── probe_store.rs
│   │       ├── baseline_store.rs
│   │       ├── run_store.rs       ← run metadata / embeddings / time-ordered index (3 tables)
│   │       ├── alert_store.rs
│   │       ├── schedule_store.rs ← per-probe next-run state (restart catch-up)
│   │       └── spend_store.rs    ← per-rule alpha-spend ledger (sequential control)
│   │
│   ├── daemon/                 ← binary: tokio runtime, scheduler, REST API
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── server.rs           ← axum router + auth middleware + state
│   │       ├── scheduler.rs        ← tokio-cron-scheduler integration
│   │       ├── constants.rs        ← daemon runtime tuning constants
│   │       ├── vault.rs            ← encrypted API key management (age)
│   │       ├── provider_factory.rs ← unified resolver: ProviderSpec → DynProvider
│   │       └── routes/
│   │           ├── mod.rs
│   │           ├── probes.rs
│   │           ├── baselines.rs
│   │           ├── runs.rs
│   │           ├── alerts.rs
│   │           └── vault.rs    ← /api/vault key management endpoints
│   │
│   └── cli/                    ← binary: clap derive CLI
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           └── commands/
│               ├── probe.rs    ← probe add / list / delete / run-now / status
│               ├── baseline.rs ← baseline capture / list
│               └── alert.rs    ← alert list / ack
│
├── web/                        ← SvelteKit frontend (Svelte 5 runes)
│   ├── package.json
│   ├── svelte.config.js
│   ├── vite.config.ts
│   ├── tsconfig.json
│   └── src/
│       ├── app.html
│       ├── app.css             ← plain CSS (no Tailwind)
│       ├── lib/
│       │   ├── api.ts          ← typed fetch wrappers with auth support
│       │   ├── types.ts        ← mirrors Rust models (manual)
│       │   └── components/
│       │       ├── AddProbeForm.svelte
│       │       ├── DriftChart.svelte
│       │       ├── DriftMetrics.svelte
│       │       ├── ProbeTable.svelte
│       │       └── SummaryCard.svelte
│       └── routes/
│           ├── +layout.svelte
│           ├── +error.svelte          ← global error boundary
│           ├── +page.svelte           ← dashboard overview
│           └── probes/
│               ├── +page.svelte
│               └── [id]/+page.svelte
│
├── config/
│   └── default.toml            ← example config; committed to repo
│
├── docs/
│   ├── ARCHITECTURE.md
│   ├── THRESHOLD_TUNING.md
│   ├── LOCAL_CI_GUIDE.md
│   └── RELEASE_READINESS_CHECKLIST.md
│
└── Makefile                    ← dev shortcuts (check, test, run, fmt)
```

---

## 3. Full Tech Stack

### Rust Backend

| Crate | Version | Purpose |
|---|---|---|
| `tokio` | 1.x | Async runtime (`full` features) |
| `axum` | 0.8 | REST API router + middleware |
| `tower` | 0.5 | Middleware layers (timeout) |
| `tower-http` | 0.6 | CORS, ServeDir, request logging |
| `hyper` | 1.x | HTTP client/server (used by axum internally) |
| `reqwest` | 0.12 | HTTP client for LLM provider API calls |
| `lettre` | 0.11 | Async SMTP client (rustls TLS) for the email alert channel |
| `serde` | 1.x | Serialization framework |
| `serde_json` | 1.x | JSON encoding/decoding |
| `toml` | 1.x | Config file parsing |
| `redb` | 4.x | Pure-Rust embedded database (no C deps) |
| `age` | 0.11 | Encryption for API key vault |
| `secrecy` | 0.10 | `SecretBox<T>` / `SecretString` wrapper — prevents accidental logging |
| `thiserror` | 2.x | Error type derive macro |
| `anyhow` | 1.x | Error propagation in binaries only (never in lib crates) |
| `tracing` | 0.1 | Structured logging + spans |
| `tracing-subscriber` | 0.3 | JSON log formatter, env-filter |
| `async-trait` | 0.1 | Async methods in trait definitions |
| `proptest` | 1.x | Property-based testing for drift algorithms |
| `criterion` | 0.8 | Micro-benchmarks |
| `clap` | 4.x | CLI with derive macros |
| `cron` | 0.16 | Cron expression parsing for probe scheduling |
| `uuid` | 1.x | ID generation (v4, features = ["v4", "serde"]) |
| `chrono` | 0.4 | Timestamps (UTC only, serde feature) |
| `mockall` | 0.14 | Mock generation for `LlmProvider` in unit tests |
| `wiremock` | 0.6 | HTTP mock server for integration tests |
| `tempfile` | 3.x | Temporary directories for test databases and vaults |
| `include_dir` | 0.7 | Embed static web assets into daemon binary |

### Svelte Frontend

| Package | Version | Purpose |
|---|---|---|
| `svelte` | 5.x | UI framework (runes: `$state`, `$derived`, `$props`) |
| `@sveltejs/kit` | 2.x | Full-stack framework (routing, SSR) |
| `vite` | 8.x | Build tool |
| `chart.js` | 4.x | Drift score timeline charts (used directly, no wrapper) |
| `zod` | 4.x | API response validation |

### Tooling

| Tool | Purpose |
|---|---|
| `cargo clippy` | Linter — `-D warnings` in CI |
| `cargo fmt` | Formatter — `--check` in CI |
| `cargo test` | Unit + integration tests |
| `cargo bench` | Benchmark suite (criterion) |
| `cargo audit` | Dependency vulnerability scan (CI `security` job) |

> See [`LOCAL_CI_GUIDE.md`](LOCAL_CI_GUIDE.md) for the full local hook / CI-equivalent workflow.

---

## 4. Project Setup

### Prerequisites

```bash
# Install Rust stable via rustup
rustup toolchain install stable
rustup component add clippy rustfmt

# Install cargo tools
cargo install cargo-audit

# Node.js 22+ for the web frontend
node --version  # >= 22.0.0
```

### rust-toolchain.toml

```toml
[toolchain]
channel = "stable"
components = ["clippy", "rustfmt"]
```

### Workspace Cargo.toml

```toml
[workspace]
resolver = "2"
members = [
    "crates/common",
    "crates/core",
    "crates/store",
    "crates/daemon",
    "crates/cli",
]

[workspace.dependencies]
tokio       = { version = "1", features = ["full"] }
axum        = { version = "0.8" }
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
thiserror   = "2"
anyhow      = "1"
tracing     = "0.1"
uuid        = { version = "1", features = ["v4", "serde"] }
chrono      = { version = "0.4", features = ["serde"] }
redb        = "4"
secrecy     = "0.10"
toml        = "1"

[workspace.lints.rust]
unsafe_code = "forbid"          # no unsafe in any crate

[workspace.lints.clippy]
all         = "warn"
pedantic    = "warn"
unwrap_used = "warn"            # no .unwrap() in library/runtime code
panic       = "warn"            # no panic! in library/runtime code
```

### .rustfmt.toml

```toml
edition = "2024"
max_width = 100
```

### .clippy.toml

```toml
msrv = "1.85"
avoid-breaking-exported-api = false
```

### Makefile

```makefile
.PHONY: check fmt fmt-check lint test audit web-install web-check ci

check:
	cargo check --workspace --all-targets

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

audit:
	cargo audit

web-install:
	cd web && npm ci

web-check:
	cd web && npm run check && npm run build

ci: fmt-check lint test audit web-check
```

### Development Config (config/default.toml)

```toml
[server]
host = "127.0.0.1"
port = 7740
timeout_secs = 30
cors_origin = "http://localhost:5173"

[vault]
path = ".modelsentry/vault"

[database]
path = ".modelsentry/store.redb"

[scheduler]
default_interval_minutes = 60

[alerts]
target_fpr = 0.01            # per-run calibrated false-positive rate; alert when p < this
baseline_capture_runs = 20   # recent runs aggregated into a baseline capture
permutations = 200           # pooled-fallback permutation count
cooldown_secs = 3600         # de-dup window: silence repeat alerts for one rule
# [alerts.sequential]        # optional sequential control (alpha-spending); off by default
# window_secs = 2592000      # rolling window (30 days)
# alpha_budget = 0.05        # bound on expected false alarms per rule per window

[providers.openai]
model = "gpt-5.4"
embedding_model = "text-embedding-3-small"
embedding_dim = 1536
base_url = "https://api.openai.com"

[providers.anthropic]
model = "claude-sonnet-4-6"
base_url = "https://api.anthropic.com"
api_version = "2023-06-01"

[providers.ollama]
model = "llama3"
base_url = "http://localhost:11434"

[providers.azure]
endpoint = ""                 # https://<resource>.openai.azure.com (required for Azure)
# embedding_deployment = "..."  # set to enable drift; omit for completions-only
embedding_dim = 1536
api_version = "2024-10-21"

[auth]
enabled = false
# api_keys = ["your-secret-key"]
```

---

## 5. Testing Strategy

### Pyramid

```
                     ┌──────────┐
                     │  E2E     │  (few; full stack: daemon + real API sandbox)
                    ┌┴──────────┴┐
                    │Integration │  (moderate; daemon routes, scheduler, redb)
                   ┌┴────────────┴┐
                   │  Unit tests  │  (many; all pure logic in crates/core, crates/common)
                   └──────────────┘
```

### Unit Tests (`crates/core`, `crates/common`)
- Co-located with source in `#[cfg(test)]` modules
- Mock `LlmProvider` via `mockall`
- Property-based tests on all drift algorithms using `proptest`
- Fast: must complete in <2s total
- Zero file I/O, zero network calls
- **Coverage target: 90%+ for `crates/core`**

### Integration Tests (`crates/core/tests/`)
- Test probe lifecycle, drift detection, and alert firing with mock providers
- Test `crates/store` against a real `redb` database opened in `tempdir`
- Route handler tests co-located in `#[cfg(test)]` modules in daemon route files
- Vault encryption round-trip tests in `crates/daemon/src/vault.rs`
- Must run in `cargo nextest` with no external setup

### Property-Based / Invariant Tests
- Every drift primitive must verify its mathematical invariants:
  - MMD²/energy: a distribution against itself yields a statistic ≈ 0
  - Permutation test: the null p-value is (approximately) uniform on `[0, 1]`,
    so thresholding at α gives FPR ≈ α (FPR-calibration test)
  - Conformal p-value: rank-valid and bounded below by `1/(cloud_size + 1)`
  - Stratified permutation gate: combined p is monotone in `T = Σ max(zᵢ, 0)`
    and resolves to `1/(B+1)` (auto-raising `B` to reach `target_fpr`)
  - Centroid: dimension preservation

### Benchmarks (criterion)
- `crates/core/benches/drift_bench.rs` — benchmark the end-to-end
  `DriftCalculator::compute` (conformal + pooled fallback) at N=100, 500, 1536
  embedding dims

### CI Test Matrix

CI runs on `ubuntu-latest` with the stable toolchain across three jobs — `rust` (check / fmt / clippy / test), `security` (cargo audit), and `frontend` (npm check / build / audit). See [`LOCAL_CI_GUIDE.md`](LOCAL_CI_GUIDE.md).

---

## 6. Compiler Check Approach (Per Commit)

### Pre-commit (local, via Makefile or git hook)

Every developer commit must pass locally before push:

```sh
# .githooks/pre-commit (enabled via `git config core.hooksPath .githooks`)
set -e
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
# svelte-check runs only when web/ files are staged
# (cargo test + audits run in the pre-push hook)
```

### CI (every push + every PR)

```yaml
# .github/workflows/ci.yml — three jobs, all on ubuntu-latest
on: [push, pull_request]

jobs:
  rust:      # cargo check / fmt --check / clippy -D warnings / test --workspace
  security:  # cargo audit
  frontend:  # npm ci / npm run check / npm run build / npm audit --audit-level=high
```

CodeQL and dependency-review run via separate workflows (`codeql.yml`, `dependency-review.yml`).

### Release Gate (tags only)
Release builds run via `.github/workflows/release.yml`. See [`RELEASE_READINESS_CHECKLIST.md`](RELEASE_READINESS_CHECKLIST.md) for the manual pre-release gate.

---

## 7. Linter Configuration

Lints are declared **once** at the workspace root (`Cargo.toml` `[workspace.lints]`) and inherited by every crate via `[lints] workspace = true` — there are no per-crate-root `#![deny(...)]` attributes to keep in sync. They are set as **warnings** and promoted to hard errors in CI and the git hooks via `cargo clippy --workspace --all-targets -- -D warnings`. No `#[allow(...)]` without a comment explaining why.

### Lints Enforced

```toml
# Cargo.toml — [workspace.lints]
[workspace.lints.rust]
unsafe_code = "forbid"        # no unsafe anywhere; cannot be re-enabled per-crate

[workspace.lints.clippy]
all         = "warn"          # the default correctness/style groups
pedantic    = "warn"          # stricter style/idiom group
unwrap_used = "warn"          # no .unwrap() in library/runtime code
panic       = "warn"          # no panic! in library/runtime code
```

`.clippy.toml` exempts **tests** from the panic-family restrictions (`allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests`), and sets `msrv = "1.85"`. `expect("reason")` remains allowed in runtime code for documented invariants (e.g. lock poisoning).

> Note: the restriction lints `clippy::indexing_slicing`, `clippy::arithmetic_side_effects`, and `clippy::float_arithmetic` are **not** enabled — the drift math uses direct float arithmetic and (sparingly) indexing by design. The code still prefers `.get()` and checked patterns where practical, but that is a convention, not a compiler-enforced rule.

### Allowed Exceptions (explicit in source, each with a reason)
- `#[allow(clippy::doc_markdown)]` — on `constants::provider` / `constants::defaults`, where brand names like "OpenAI" trip the backtick lint.
- `#[allow(clippy::cast_precision_loss)]` — at `usize → f32/f64` sites in the drift math/tests, where the counts are small and the loss is irrelevant.
- `#[allow(clippy::module_name_repetitions)]` — on store structs such as `ProbeStore`.

---

## 8. Code Formatting Rules

Enforced by `rustfmt` automatically. Non-negotiable:

- Line length: **100 characters**
- Trailing commas: always in multi-line constructs
- Import grouping: `std` → external crates → internal crates (enforced by `group_imports`)
- No `use super::*`; always explicit imports
- Blank line between every item in `impl` blocks
- One blank line at end of file, no trailing whitespace
- `match` arms: always exhaustive; no `_ => unreachable!()` without `#[non_exhaustive]` justification

---

## 9. Coding Style Guidelines & Rules

### Newtype Pattern — Mandatory for All IDs

```rust
// ✅ correct
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProbeId(Uuid);

impl ProbeId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

// ❌ wrong — stringly typed IDs are forbidden
fn get_probe(probe_id: &str) -> Result<Probe, Error> { ... }
```

### Error Handling

```rust
// Library crates: thiserror only — never anyhow
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("provider returned non-200 status: {status}")]
    ProviderHttp { status: u16 },
    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
}

// Binary crates (daemon/main.rs, cli/main.rs): anyhow is allowed
// All intermediate layers: thiserror with structured variants

// ✅ never: .unwrap(), panic!(), todo!() in library code
// ✅ always: return Err(...) with a structured variant
```

### Secret Handling

```rust
use secrecy::{SecretString, ExposeSecret};

// API keys are always wrapped in SecretString
pub struct OpenAiProvider {
    api_key: SecretString,
    client: reqwest::Client,
}

// Expose only at the precise call site:
let key = self.api_key.expose_secret();
// Never: format!("{}", self.api_key.expose_secret())  ← log risk
```

### Async Trait Adapters

```rust
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, CoreError>;
    async fn complete(&self, prompt: &str) -> Result<String, CoreError>;
    fn provider_name(&self) -> &'static str;
}
```

### Struct Construction — Builder or Direct

```rust
// For structs with >4 fields: use the typed-builder or manual builder pattern
// For structs with ≤4 fields: direct construction is fine
// Never: Default::default() + field mutation after construction
```

### Logging — Structured Only

```rust
// ✅ structured spans + events
tracing::info!(probe_id = %id, provider = %provider_name, "probe run started");
tracing::error!(error = %e, probe_id = %id, "probe run failed");

// ❌ string interpolation in log messages
println!("probe {} ran", id);
log::info!("error: {}", e);
```

### No `pub` Without Justification

Every `pub` item in a library crate must be part of the public API contract. Items used only within a crate are `pub(crate)`. Items used only within a module are private.

---

## 10. Anti-Patterns to Avoid

| Anti-Pattern | Why | Correct Alternative |
|---|---|---|
| `.unwrap()` in library code | Panics propagate silently across crate boundaries | Return `Result<T, E>` with a structured error variant |
| `String` for IDs | Allows passing wrong IDs, silent bugs | Newtype wrapper over `Uuid` |
| `Vec<f32>` as a function boundary for embeddings | No dimension safety | Newtype `Embedding(Vec<f32>)` with checked constructors |
| Global mutable state (`static mut`, `lazy_static` with `Mutex<Option<T>>`) | Hidden coupling, impossible to test in isolation | Dependency injection via struct fields |
| `async fn` in a `trait` without `async-trait` | Silent Send bound issues | `#[async_trait]` macro explicitly |
| Storing raw API keys in config struct | Leak risk via Debug/log | `Secret<String>` + `age`-encrypted vault |
| `HashMap<String, serde_json::Value>` for typed domain models | Loses compile-time checks | Explicit `struct` with `serde` |
| Deep `match` nesting without helper functions | Readability collapses | Extract named predicate functions |
| Large monolithic `async fn` in route handlers | Untestable | Extract business logic into `core` crate functions called from handler |
| Direct `redb` calls in route handlers | Bypasses store abstraction | All DB access via `crates/store` public API only |
| Panic on misconfiguration at startup | Silent failure in prod | Validate config at startup, return `Err`, log clearly, exit code 1 |

---

## 11. Bad Smells to Detect and Eliminate

These will be caught in code review if not caught by clippy:

1. **God struct** — a single struct that holds application config, database handle, HTTP client, and scheduler. Split into focused structs; compose via `AppState`.

2. **Implicit string parsing deep in call chain** — e.g., parsing a UUID from a string in a route handler body rather than extracting via typed `Path<ProbeId>` in axum.

3. **Shadowed error types** — converting a specific error to `anyhow::Error` inside a library function, losing the structured type at the boundary. Library errors must stay typed.

4. **Non-deterministic test ordering** — tests that share state via a global/static and depend on execution order. Each test must create its own `tempdir` and `redb` instance.

5. **Magic numbers** — embedding dimension hardcoded as `1536` anywhere outside the provider adapter. Use named constants.

6. **Float equality comparison** — `if mmd_stat == 0.0` in drift logic. Use `< f32::EPSILON` with documentation explaining the tolerance.

7. **Unused `Result` discards** — any `let _ = fallible_fn()` without an explicit comment justifying why the error is intentionally ignored.

8. **Missing timeout on external HTTP calls** — all `reqwest` calls to LLM provider APIs must have an explicit `.timeout(Duration::from_secs(N))`.

9. **Clone-happy code** — cloning large `Vec<Vec<f32>>` (embedding matrices) unnecessarily. Prefer passing `&[Vec<f32>]`.

10. **Implicit unwrap via `From` impl** — implementing `From<Option<T>>` for a domain type that panics on `None`. Always explicit.

---

## 12. Security Requirements

- **API keys**: never appear in logs, debug output, or error messages. Enforced by `secrecy::Secret`.
- **Vault**: `age` file encryption with a key derived from a user-provided passphrase. Key is never stored on disk unencrypted.
- **HTTP API**: authentication via `Authorization: Bearer <key>` or `X-Api-Key` header, configurable in `[auth]` config section. CORS origin is config-driven (defaults to `http://localhost:5173`).
- **Input validation**: all API request bodies validated before processing. `serde` deserialization errors return `422 Unprocessable Entity`, never `500`.
- **No `unsafe` in any crate**: enforced by `[workspace.lints.rust] unsafe_code = "forbid"` inherited by all crates.
- **Dependency audit**: `cargo audit` and `cargo deny` run on every CI push.
- **TLS**: all outbound LLM API requests use `rustls-tls` (no `native-tls`). Reqwest default-features disabled.
