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
│   │       │   ├── ollama.rs
│   │       │   └── azure.rs
│   │       ├── drift/
│   │       │   ├── mod.rs
│   │       │   ├── kl.rs       ← kl_divergence(), gaussian_kl()
│   │       │   ├── cosine.rs   ← cosine_distance(), centroid()
│   │       │   └── entropy.rs  ← output_entropy()
│   │       ├── probe_runner.rs ← ProbeRunner struct
│   │       ├── baseline.rs     ← BaselineSnapshot, snapshot computation
│   │       └── alert.rs        ← AlertEvaluator, threshold check
│   │
│   ├── store/                  ← persistence layer (redb)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── db.rs           ← open_db(), migrations
│   │       ├── probe_store.rs
│   │       ├── baseline_store.rs
│   │       ├── run_store.rs
│   │       └── alert_store.rs
│   │
│   ├── daemon/                 ← binary: tokio runtime, scheduler, REST API
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── server.rs       ← axum router + state
│   │       ├── scheduler.rs    ← tokio-cron-scheduler integration
│   │       ├── vault.rs        ← encrypted API key management (age)
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
├── web/                        ← SvelteKit frontend
│   ├── package.json
│   ├── svelte.config.js
│   ├── vite.config.ts
│   ├── tailwind.config.ts
│   ├── tsconfig.json
│   └── src/
│       ├── app.html
│       ├── app.css
│       ├── lib/
│       │   ├── api.ts          ← typed fetch wrappers for REST API
│       │   ├── types.ts        ← mirrors Rust models (generated or manual)
│       │   └── components/
│       │       ├── DriftChart.svelte
│       │       ├── ProbeTable.svelte
│       │       ├── BaselineBadge.svelte
│       │       └── AlertFeed.svelte
│       └── routes/
│           ├── +layout.svelte
│           ├── +page.svelte           ← dashboard overview
│           ├── probes/
│           │   ├── +page.svelte
│           │   └── [id]/+page.svelte
│           └── baselines/
│               └── +page.svelte
│
├── config/
│   └── default.toml            ← example config; committed to repo
│
├── tests/
│   └── integration/
│       ├── probe_lifecycle.rs
│       ├── drift_detection.rs
│       └── alert_fire.rs
│
└── .github/
    └── workflows/
        ├── ci.yml
        └── release.yml
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
| `indicatif` | 0.17 | Progress bars in CLI |
| `insta` | 1.x | Snapshot testing for API responses |
| `mockall` | 0.12 | Mock generation for `LlmProvider` in unit tests |
| `testcontainers` | 0.15 | Integration test infrastructure |

### Svelte Frontend

| Package | Version | Purpose |
|---|---|---|
| `svelte` | 5.x | UI framework |
| `@sveltejs/kit` | 2.x | Full-stack framework (routing, SSR) |
| `vite` | 7.x | Build tool |
| `tailwindcss` | 3.x | Utility-first styling |
| `chart.js` | 4.x | Drift score timeline charts |
| `svelte-chartjs` | 2.x | Svelte wrapper for Chart.js |
| `@tanstack/svelte-table` | 8.x | Probe run history table |
| `svelte-sonner` | 1.x | Toast notifications for alerts |
| `zod` | 3.x | API response validation |

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

[vault]
path = ".modelsentry/vault"

[database]
path = ".modelsentry/store.redb"

[scheduler]
default_interval_minutes = 60

[alerts]
drift_threshold_kl = 0.1
drift_threshold_cos = 0.15
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

### Integration Tests (`tests/integration/`)
- Test daemon HTTP routes via `axum::test` helpers (in-process, no real network)
- Test `crates/store` against a real `redb` database opened in `tempdir`
- Use `testcontainers` only if a real external service is absolutely required
- Must run in `cargo nextest` with no external setup

### Property-Based Tests (proptest)
- Every drift algorithm must have at least one proptest verifying mathematical invariants:
  - KL divergence: `kl(p, p) == 0.0`
  - KL divergence: `kl(p, q) >= 0.0` for all valid p, q
  - Cosine distance: `cosine_distance(v, v) == 0.0`
  - Cosine distance: range `[0.0, 1.0]`
  - Centroid: dimension preservation

### Snapshot Tests (insta)
- All API response JSON shapes tested with `insta::assert_json_snapshot!`
- Run `cargo insta review` after intentional shape changes

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
- **HTTP API**: no authentication in v1 (localhost-only binding). v2 will add bearer token.
- **Input validation**: all API request bodies validated before processing. `serde` deserialization errors return `422 Unprocessable Entity`, never `500`.
- **No `unsafe` in library crates**: enforced by `#![forbid(unsafe_code)]` in `crates/common`, `crates/core`, `crates/store`. `unsafe` is only permitted in `crates/daemon` if required by FFI, with a `SAFETY` comment on every block.
- **Dependency audit**: `cargo audit` and `cargo deny` run on every CI push.
- **TLS**: all outbound LLM API requests use `rustls-tls` (no `native-tls`). Reqwest default-features disabled.

---

## 13. Commercialization Architecture (Must-Have)

These requirements are mandatory for converting ModelSentry from a technical project into a sellable product.

### 13.1 Ideal Customer Profile (ICP)

- B2B teams with production LLM workflows (classification, extraction, routing, support automation)
- Engineering organizations with 20-300 engineers
- Teams already using incident tooling (PagerDuty, Sentry, Datadog)
- Teams with explicit reliability ownership (platform, AI infra, applied ML)

### 13.2 Positioning

ModelSentry is not "general LLM observability." It is:

- A production risk-control system for silent model behavior shifts
- A regression early-warning system before KPI or workflow breakage
- A compliance-friendly monitoring layer for model change evidence

### 13.3 Packaging Strategy

- OSS core: probe execution, baseline diffing, local dashboard, webhook alerts
- Paid cloud/control-plane features:
  - SSO/SAML/OIDC + RBAC
  - long-term retention and cross-environment comparison
  - business KPI connectors (conversion, extraction accuracy, support resolution)
  - advanced alert routing and escalation policies
  - immutable audit exports for governance

### 13.4 Pricing Axis

Primary value metric:

- monitored endpoints
- monthly probe volume

Secondary controls:

- retention window
- team seats / SSO entitlement
- private network deployment option

---

## 14. Core Design Optimizations

### 14.1 Provider-Aware Metric Pipelines

Problem: providers do not expose identical capabilities (for example, no native embedding endpoint in Anthropic).

Requirement:

- Add `MetricPipeline` abstraction selected per provider capability:
  - `EmbeddingMetricsPipeline`: centroid drift, cosine distance, KL distribution shifts
  - `TextOnlyPipeline`: entropy delta, schema/format constraints, rubric checks

Rules:

- No probe may claim embedding-based scoring if provider capability does not support it
- All API/UI responses must expose `metric_capability` so users understand why a score exists or is absent

### 14.2 False-Positive Suppression Layer

Problem: alert fatigue kills product trust and retention.

Requirement:

- Add `AlertStabilityPolicy` with:
  - warmup runs before alerts are enabled
  - N-consecutive breach rule
  - rolling window percentile threshold mode
  - cooldown window after alert fire
  - confidence score in every alert payload

### 14.3 Baseline Lifecycle

Requirement:

- Baselines have explicit lifecycle states:
  - `candidate`
  - `active`
  - `retired`
- Add promotion workflow:
  - candidate created from a run
  - manual or policy-based promotion to active
  - automatic retirement of replaced baseline

### 14.4 Cost Governance

Requirement:

- Add probe budget controls:
  - monthly token cap per probe
  - adaptive sampling (lower frequency when stable)
  - high-risk burst mode (temporarily higher frequency on anomaly)
  - hard stop when budget exhausted

---

## 15. Architecture Delta (From v1 to Sellable v1.5)

The following components must be added to the original system diagram:

- `Capability Resolver`: maps provider/model to supported metrics
- `Metric Pipeline Router`: chooses embedding or text-only pipeline
- `Baseline Lifecycle Manager`: candidate/active/retired state machine
- `Alert Stability Engine`: suppresses noise and computes confidence
- `Budget Controller`: enforces spend and adaptive schedule policy
- `Impact Correlator`: optional module correlating drift events with business KPI drops

Data model additions:

- `Probe`: `budget_policy`, `stability_policy`, `metric_capability`
- `BaselineSnapshot`: `state`, `promoted_from_run_id`, `retired_at`
- `DriftReport`: `confidence`, `pipeline_used`, `explanation`
- `AlertEvent`: `suppressed_reason`, `policy_version`

---

## 16. Sellability Risks and Mitigations

1. Risk: False alarms reduce trust.
Mitigation: stability policies, confidence score, suppression reason visibility.

2. Risk: "Drift" not tied to business value.
Mitigation: KPI correlation module and dashboard panel showing drift-to-impact lag.

3. Risk: Procurement blockers in mid/enterprise.
Mitigation: SSO/RBAC/audit export architecture hooks from day one.

4. Risk: Cost unpredictability from probe traffic.
Mitigation: budget controller, adaptive sampling, hard caps, spend forecasting.

5. Risk: Capability mismatch across model providers.
Mitigation: provider-aware metric routing and explicit per-metric capability exposure.
