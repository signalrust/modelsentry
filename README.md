# ModelSentry

ModelSentry is a Rust-first and Svelte-first platform for detecting silent behavior drift in LLM endpoints.

This repository is structured as an enterprise-oriented foundation: strict quality gates, explicit architecture documentation, and a phased implementation plan.

## Project Status

**Phase 0 (Scaffold)** — complete:
- Rust workspace compiles with all crate skeletons.
- Frontend workspace (`web/`) initialized with SvelteKit 2 + Svelte 5, passes type checks.
- CI workflow present for Rust quality gates.
- Architecture and execution plan documented in `docs/`.

**Phase 1 (Common Types)** — complete:
- Newtype ID types (`ProbeId`, `BaselineId`, `RunId`, `AlertRuleId`) via `define_id!` macro.
- `ApiKey` with `secrecy::SecretString` — redacted in Debug, Serialize, and logs.
- Domain error hierarchy (`ModelSentryError`) with `thiserror`.
- All domain models: `Probe`, `BaselineSnapshot`, `ProbeRun`, `DriftReport`, `AlertRule`, `AlertEvent`.
- `AppConfig` with TOML loading and validation.
- 32 unit tests passing.

Not yet implemented:
- Drift algorithms and provider integrations (Phase 2).
- Persistence layer and API routes (Phase 3+).
- Production operational features.

## Design Intent

The project is intentionally designed to match expectations for professional teams using platforms like OpenAI, Anthropic, and Google model APIs:

- Rust backend with strong static guarantees and explicit linting/formatting standards.
- SvelteKit frontend with strict type checking and reproducible build pipeline.
- Documentation-driven delivery: architecture, standards, and task-level plan live in the repo.
- Enterprise trajectory in docs: reliability controls, baseline lifecycle, and commercialization requirements.

## Repository Layout

```text
modelsentry/
├── Cargo.toml                 # Rust workspace
├── rust-toolchain.toml        # Stable toolchain + components
├── Makefile                   # Common developer commands
├── config/default.toml        # Runtime defaults (host, port, db, scheduler)
├── crates/
│   ├── common/                # Shared types/errors (scaffold)
│   ├── core/                  # Core logic (scaffold)
│   ├── store/                 # Persistence layer (scaffold)
│   ├── daemon/                # Backend binary (scaffold)
│   └── cli/                   # CLI binary (scaffold)
├── web/                       # SvelteKit application
└── docs/
		├── ARCHITECTURE.md
		└── PROJECT_PLAN.md
```

## Technology Baseline

### Backend (Rust)

- Workspace resolver: `2`
- Edition: `2024`
- Toolchain: stable (`rust-toolchain.toml`)
- Key dependencies: `serde 1`, `tokio 1`, `axum 0.8`, `redb 3`, `thiserror 2`, `secrecy 0.10`, `toml 1`, `chrono 0.4`, `uuid 1`
- Lint policy in workspace (enforced via `[lints] workspace = true` in all crates):
	- `unsafe_code = "forbid"`
	- `clippy::all` and `clippy::pedantic` enabled as warnings

### Frontend (Svelte)

- `@sveltejs/kit` with Vite
- TypeScript strict checks via `svelte-check`
- Dependency override present for secure transitive resolution (`cookie`)

## Quality and Verification

### Rust CI Gates

Workflow: `.github/workflows/ci.yml`

- `cargo check --workspace --all-targets`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo audit`
- frontend `npm ci`, `npm run check`, `npm run build`, `npm audit --audit-level=high`

### Local Developer Commands

From repository root:

```bash
make check
make fmt-check
make lint
make test
```

### Frontend Checks

From `web/`:

```bash
npm install
npm run check
npm run build
```

## Getting Started

### Prerequisites

- Rust stable + components `clippy`, `rustfmt`
- Node.js + npm for frontend tasks

### 1) Backend workspace

```bash
cargo check --workspace --all-targets
```

### 2) Frontend workspace

```bash
cd web
npm install
npm run check
```

### 3) Default config reference

`config/default.toml` currently defines:

- Server host/port (`127.0.0.1:7740`)
- Embedded database path (`.modelsentry/store.redb`)
- Scheduler default interval (`60` minutes)

## Documentation and Source of Truth

- Architecture and engineering standards: `docs/ARCHITECTURE.md`
- Step-by-step implementation plan: `docs/PROJECT_PLAN.md`
- Release gate checklist: `docs/RELEASE_READINESS_CHECKLIST.md`

When implementation details and README differ, `docs/ARCHITECTURE.md` and `docs/PROJECT_PLAN.md` are the authoritative planning documents.

## Engineering Principles

- Do not overclaim readiness: production claims are only valid once corresponding code and tests exist.
- Keep changes incremental and testable: each task should have explicit definition of done.
- Enforce deterministic quality gates in CI before merge.
- Favor explicit types and boundaries over implicit behavior.
- Keep security hygiene continuous: run `npm audit` and Rust dependency checks as part of normal maintenance.

## License

Business Source License 1.1 (`LICENSE`).

Self-hosted use for your own internal purposes is unrestricted. Running
ModelSentry as a hosted service for third parties requires a commercial
license. The license converts to Apache 2.0 on 2030-03-06.

## Public Repo Governance

This repository includes enterprise baseline governance files:

- Security policy: `SECURITY.md`
- Contribution guide: `CONTRIBUTING.md`
- Code of conduct: `CODE_OF_CONDUCT.md`
- Changelog policy: `CHANGELOG.md`
- Automated dependency updates: `.github/dependabot.yml`
