# ModelSentry

**Self-hosted LLM drift detection.** ModelSentry continuously probes your LLM endpoints, captures statistical baselines, and fires alerts the moment model behaviour shifts — all without sending data to a third-party service.

[![CI](https://github.com/signalrust/modelsentry/actions/workflows/ci.yml/badge.svg)](https://github.com/signalrust/modelsentry/actions/workflows/ci.yml)

---

## Quickstart (Docker Compose)

The fastest way to run ModelSentry locally — no Rust toolchain required.

```bash
git clone https://github.com/signalrust/modelsentry.git
cd modelsentry

# Copy and edit the config — at minimum set your OpenAI key
cp config/default.toml config/local.toml
$EDITOR config/local.toml

# Start daemon + dashboard
docker compose up
```

The dashboard opens at **http://localhost:5173**.  
The REST API is available at **http://localhost:7740/api**.

---

## Quickstart (from source)

### Prerequisites

| Tool | Version |
|------|---------|
| Rust | stable (see `rust-toolchain.toml`) |
| Node.js | ≥ 22 |
| npm | ≥ 10 |

### 1 — Install binaries

```bash
# Daemon (REST API + scheduler)
cargo install --path crates/daemon

# CLI
cargo install --path crates/cli
```

### 2 — Configure

```bash
cp config/default.toml config/local.toml
```

Edit `config/local.toml` to set your database and vault paths (defaults write to `.modelsentry/` in the current directory).

The encrypted vault is unlocked at daemon startup with a passphrase supplied via the `MODELSENTRY_VAULT_PASSPHRASE` environment variable (or the `--vault-passphrase` flag). API keys are stored through the REST API — ModelSentry never logs or writes them to disk unencrypted:

```bash
# With the daemon running, store your OpenAI key:
curl -X PUT http://127.0.0.1:7740/api/vault/keys/openai \
  -H 'content-type: application/json' \
  -d '{"key": "sk-..."}'
```

`provider` is the path segment (`openai`, `anthropic`, `ollama`). The body also accepts optional `model` and `base_url` overrides. For Ollama, `key` may be empty.

### 3 — Start the daemon

```bash
modelsentry-daemon --config config/local.toml
# Listening on http://127.0.0.1:7740
```

### 4 — Create your first probe

Create a TOML file describing the probe:

```toml
# my_probe.toml
name         = "gpt-5-smoke"
provider     = "open_ai"
model        = "gpt-5.4"
schedule     = { kind = "every_minutes", minutes = 60 }

[[prompts]]
text = "Summarise the theory of relativity in one sentence."

[[prompts]]
text = "What is the capital of France?"
```

Add it via the CLI:

```bash
modelsentry probe add --config my_probe.toml
# Created probe  id = 018e1234-...
```

### 5 — Capture a baseline

Run the probe once and lock in the statistical baseline that all future runs will be compared against:

```bash
modelsentry probe run-now 018e1234-...
modelsentry baseline capture 018e1234-...
```

### 6 — Watch for drift

ModelSentry runs the probe on the configured schedule. Check the latest drift metrics at any time:

```bash
modelsentry probe status 018e1234-...
```

```
Probe      gpt-5-smoke
Status     ok
Drift      None  (KL 0.003 / Cos 0.001)
Last run   2026-03-06 12:00 UTC (42 s)
Baseline   2026-03-06 11:45 UTC
```

### 7 — Set up an alert

```bash
modelsentry alert create \
  --probe 018e1234-... \
  --kl-threshold 0.10 \
  --cos-threshold 0.15 \
  --webhook https://hooks.slack.com/services/...
```

When drift exceeds either threshold the webhook fires with a JSON payload containing the full `DriftReport`.

---

## How It Works

```
┌──────────────────────────────────────────────────────────────┐
│  Scheduler (tokio)                                           │
│  ┌──────────┐  run()  ┌──────────────┐  DriftReport         │
│  │  Probe   │ ──────► │ ProbeRunner  │ ──────────────────►  │
│  │ (config) │         │ (LLM calls)  │    DriftCalculator   │
│  └──────────┘         └──────────────┘    (KL + cosine +    │
│                                            entropy delta)    │
│                                               │              │
│                                         AlertEngine         │
│                                         (webhook / Slack)   │
└──────────────────────────────────────────────┴──────────────┘
         │ REST API (axum)                      │ redb storage
         ▼                                      ▼
   SvelteKit Dashboard                   .modelsentry/store.redb
```

Three drift metrics are computed on every run and compared against the captured baseline:

| Metric | What it measures |
|--------|-----------------|
| **KL divergence** (Gaussian) | Shift in the embedding distribution's mean and variance |
| **Cosine distance** | Directional drift of the mean embedding from the baseline centroid |
| **Entropy delta** | Change in output token distribution entropy (vocabulary breadth) |

The `DriftLevel` is derived from the worst metric relative to configured thresholds:

| Level | Condition |
|-------|-----------|
| `None` | Both metrics within threshold |
| `Low` | 1× threshold |
| `Medium` | 2× threshold |
| `High` | 4× threshold |
| `Critical` | 8× threshold |

---

## Supported Providers

| Provider | Embeddings | Completions |
|----------|-----------|-------------|
| OpenAI | ✓ (`text-embedding-3-small`) | ✓ (`gpt-5.4`, `gpt-5.5`, etc.) |
| Anthropic | — (not supported by API) | ✓ (`claude-*`) |
| Ollama (local) | ✓ | ✓ |

---

## Repository Layout

```text
modelsentry/
├── Cargo.toml                # Rust workspace (5 crates)
├── rust-toolchain.toml       # Stable toolchain pin
├── Makefile                  # Developer shortcuts
├── config/default.toml       # Runtime defaults
├── docker-compose.yml        # One-command local setup
├── crates/
│   ├── common/               # Shared types, errors, domain models
│   ├── core/                 # Drift algorithms, provider adapters, probe runner
│   ├── store/                # redb persistence layer
│   ├── daemon/               # axum REST API + tokio scheduler binary
│   └── cli/                  # modelsentry CLI binary
├── web/                      # SvelteKit dashboard
└── docs/
    ├── ARCHITECTURE.md            # System design + engineering standards
    ├── LOCAL_CI_GUIDE.md          # Local hooks / CI-equivalent checks
    └── RELEASE_READINESS_CHECKLIST.md
```

---

## Development

### Run all quality gates locally

```bash
make check      # cargo check --workspace --all-targets
make fmt-check  # cargo fmt --all -- --check
make lint       # cargo clippy --workspace --all-targets -- -D warnings
make test       # cargo test --workspace
```

### Run benchmarks

```bash
cargo bench -p modelsentry-core
```

Criterion HTML reports are written to `target/criterion/`.

### Frontend dev server

```bash
cd web
npm ci
npm run dev      # http://localhost:5173 (proxies /api to :7740)
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
- Local CI / git-hook checks: `docs/LOCAL_CI_GUIDE.md`
- Release gate checklist: `docs/RELEASE_READINESS_CHECKLIST.md`

When implementation details and README differ, `docs/ARCHITECTURE.md` is the authoritative design document.

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
