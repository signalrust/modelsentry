# ModelSentry — Architecture & Engineering Standards
**Stack:** Rust (workspace) · Axum · Redb · Svelte 5 · SvelteKit  
**Date:** March 6, 2026

---

## 1. System Overview

ModelSentry is a self-hosted daemon that fingerprints LLM API behavior by periodically sending a fixed probe corpus to a configured endpoint, storing the resulting embedding distributions and output classifications, and alerting when any statistical metric drifts beyond a configurable threshold relative to a frozen baseline.

```
┌──────────────────────────────────────────────────────────────────────┐
│                         ModelSentry Daemon                           │
│                                                                      │
│  ┌────────────┐   schedule   ┌─────────────┐   embed/complete        │
│  │  Scheduler │─────────────▶│ Probe Runner│──────────────────────▶  │
│  │ (cron/tick)│              │             │  LLM Provider API        │
│  └────────────┘              └──────┬──────┘  (OpenAI / Anthropic /  │
│                                     │          Ollama / Azure)        │
│                              results│                                 │
│                                     ▼                                 │
│                          ┌──────────────────┐                        │
│                          │  Drift Calculator │                        │
│                          │  (KL, cosine,     │                        │
│                          │   entropy)        │                        │
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
│  │  /api/probes  /api/baselines  /api/runs  /api/alerts          │   │
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
│   │       │   └── ollama.rs
│   │       ├── drift/
│   │       │   ├── mod.rs
│   │       │   ├── calculator.rs ← DriftCalculator, DriftReport
│   │       │   ├── kl.rs       ← kl_divergence(), gaussian_kl()
│   │       │   ├── cosine.rs   ← cosine_distance(), centroid()
│   │       │   └── entropy.rs  ← output_entropy()
│   │       ├── probe_runner.rs ← ProbeRunner struct
│   │       └── alert.rs        ← AlertEngine, webhook firing
│   │
│   ├── store/                  ← persistence layer (redb)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs          ← AppStore, open_db()
│   │       ├── probe_store.rs
│   │       ├── baseline_store.rs
│   │       ├── run_store.rs
│   │       └── alert_store.rs
│   │
│   ├── daemon/                 ← binary: tokio runtime, scheduler, REST API
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── server.rs           ← axum router + auth middleware + state
│   │       ├── scheduler.rs        ← tokio-cron-scheduler integration
│   │       ├── vault.rs            ← encrypted API key management (age)
│   │       ├── provider_factory.rs ← shared provider construction
│   │       └── routes/
│   │           ├── mod.rs
│   │           ├── probes.rs
│   │           ├── baselines.rs
│   │           ├── runs.rs
│   │           └── alerts.rs
│   │
│   └── cli/                    ← binary: clap derive CLI
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           └── commands/
│               ├── probe.rs    ← probe add / list / delete / run-now
│               ├── baseline.rs ← baseline capture / diff / list
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
│   ├── AUDIT_REPORT.md
│   └── PROJECT_PLAN.md
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
| `tower` | 0.4 | Middleware layers (rate-limit, timeout, trace) |
| `tower-http` | 0.5 | CORS, ServeDir, request logging |
| `hyper` | 1.x | HTTP client/server (used by axum internally) |
| `reqwest` | 0.12 | HTTP client for LLM provider API calls |
| `serde` | 1.x | Serialization framework |
| `serde_json` | 1.x | JSON encoding/decoding |
| `toml` | 1.x | Config file parsing |
| `redb` | 3.x | Pure-Rust embedded database (no C deps) |
| `age` | 0.10 | Encryption for API key vault |
| `secrecy` | 0.10 | `SecretBox<T>` / `SecretString` wrapper — prevents accidental logging |
| `thiserror` | 2.x | Error type derive macro |
| `anyhow` | 1.x | Error propagation in binaries only (never in lib crates) |
| `tracing` | 0.1 | Structured logging + spans |
| `tracing-subscriber` | 0.3 | JSON log formatter, env-filter |
| `async-trait` | 0.1 | Async methods in trait definitions |
| `proptest` | 1.x | Property-based testing for drift algorithms |
| `criterion` | 0.5 | Micro-benchmarks |
| `clap` | 4.x | CLI with derive macros |
| `tokio-cron-scheduler` | 0.10 | Cron-style probe scheduling |
| `uuid` | 1.x | ID generation (v4, features = ["v4", "serde"]) |
| `chrono` | 0.4 | Timestamps (UTC only, serde feature) |
| `mockall` | 0.12 | Mock generation for `LlmProvider` in unit tests |
| `wiremock` | 0.6 | HTTP mock server for integration tests |
| `tempfile` | 3.x | Temporary directories for test databases and vaults |
| `include_dir` | 0.7 | Embed static web assets into daemon binary |

### Svelte Frontend

| Package | Version | Purpose |
|---|---|---|
| `svelte` | 5.x | UI framework (runes: `$state`, `$derived`, `$props`) |
| `@sveltejs/kit` | 2.x | Full-stack framework (routing, SSR) |
| `vite` | 7.x | Build tool |
| `chart.js` | 4.x | Drift score timeline charts (used directly, no wrapper) |
| `zod` | 4.x | API response validation |

### Tooling

| Tool | Purpose |
|---|---|
| `cargo clippy` | Linter — `deny(warnings)` in CI |
| `cargo fmt` | Formatter — `--check` in CI |
| `cargo test` | Unit + integration tests |
| `cargo bench` | Benchmark suite |
| `cargo audit` | Dependency vulnerability scan |
| `cargo deny` | License compliance + duplicate detection |
| `cargo udeps` | Detect unused dependencies |
| `cargo nextest` | Faster parallel test runner |

---

## 4. Project Setup

### Prerequisites

```bash
# Install Rust stable via rustup
rustup toolchain install stable
rustup component add clippy rustfmt

# Install cargo tools
cargo install cargo-nextest cargo-audit cargo-deny cargo-udeps

# Node.js 20+ for the web frontend
node --version  # >= 20.0.0
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
redb        = "3"
secrecy     = "0.10"
toml        = "1"

[workspace.lints.rust]
unsafe_code = "forbid"          # escalate to forbid; core crate gets one override

[workspace.lints.clippy]
all      = "warn"
pedantic = "warn"
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
drift_threshold_kl = 0.10
drift_threshold_cos = 0.15

[providers.openai]
model = "gpt-4o"
embedding_model = "text-embedding-3-small"
embedding_dim = 1536
base_url = "https://api.openai.com"

[providers.anthropic]
model = "claude-sonnet-4-20250514"
base_url = "https://api.anthropic.com"
api_version = "2023-06-01"

[providers.ollama]
model = "llama3"
base_url = "http://localhost:11434"

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

### Property-Based Tests (proptest)
- Every drift algorithm must have at least one proptest verifying mathematical invariants:
  - KL divergence: `kl(p, p) == 0.0`
  - KL divergence: `kl(p, q) >= 0.0` for all valid p, q
  - Cosine distance: `cosine_distance(v, v) == 0.0`
  - Cosine distance: range `[0.0, 1.0]`
  - Centroid: dimension preservation

### Benchmarks (criterion)
- `crates/core/benches/drift_bench.rs` — benchmark all three drift functions at N=100, 500, 1000 embedding dims

### CI Test Matrix

```yaml
# .github/workflows/ci.yml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest]
    rust: [stable]
```

---

## 6. Compiler Check Approach (Per Commit)

### Pre-commit (local, via Makefile or git hook)

Every developer commit must pass locally before push:

```bash
# .git/hooks/pre-commit (or via lefthook / pre-commit tool)
set -e
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib  # unit tests only (fast)
```

### CI (every push + every PR)

```yaml
# .github/workflows/ci.yml
name: CI
on: [push, pull_request]

jobs:
  ci:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - name: Cache
        uses: Swatinem/rust-cache@v2

      - name: Format check
        run: cargo fmt --all -- --check

      - name: Clippy (deny warnings)
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: Tests
        run: cargo test --workspace

      - name: Audit
        run: cargo audit

      - name: Deny (license + duplicates)
        run: cargo deny check
```

### Release Gate (tags only)
Additional steps on tag push: `cargo udeps`, `cargo bench` (comparison), frontend build.

---

## 7. Linter Configuration

All active at `--deny warnings` level in CI. No exceptions without an explicit `#[allow(...)]` with a comment explaining why.

### Clippy Lints Enforced

```rust
// At crate root of each library crate:
#![deny(
    clippy::all,
    clippy::pedantic,
    clippy::cargo,
    // specific high-value extras:
    clippy::unwrap_used,         // use ? or expect("reason") instead
    clippy::expect_used,         // only allowed in tests and main.rs
    clippy::panic,               // no panic! in library code
    clippy::indexing_slicing,    // use .get() with explicit error handling
    clippy::arithmetic_side_effects,  // checked arithmetic in drift math
    clippy::float_arithmetic,    // annotate intentional float math
    clippy::missing_errors_doc,  // document error conditions in pub fns
)]
#![warn(
    clippy::nursery,
    clippy::missing_panics_doc,
)]
// Binary crates (daemon, cli) are less strict:
// allow expect_used in main.rs only
```

### Allowed Exceptions (explicit in source)
- `#[allow(clippy::module_name_repetitions)]` — on structs like `ProbeStore` in `probe_store.rs`
- `#[allow(clippy::float_arithmetic)]` — on functions in `crates/core/drift/` with a comment citing the formula

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

6. **Float equality comparison** — `if kl_score == 0.0` in drift logic. Use `< f32::EPSILON` with documentation explaining the tolerance.

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
