# ModelSentry

Your prompts don't change. The provider swaps a model version, patches a
jailbreak, reroutes you to a distilled variant, or changes a system prompt
upstream — and the API keeps returning `200`. Nothing errors. You find out
from users, not logs.

ModelSentry watches for that. It runs a fixed probe corpus against your LLM
endpoint on a schedule, embeds the completions, and runs a calibrated
nonparametric two-sample test (conformal prediction + MMD/energy-distance
permutation testing) against a stored baseline. Alerts fire on a p-value
threshold you choose, not a hand-tuned similarity score — `target_fpr = 0.01`
means what it says. Runs self-hosted, as a single Rust binary plus an
embedded database; nothing leaves your infrastructure.

[![CI](https://github.com/signalrust/modelsentry/actions/workflows/ci.yml/badge.svg)](https://github.com/signalrust/modelsentry/actions/workflows/ci.yml)

See [`docs/DRIFT_DETECTION_METHODOLOGY.md`](docs/DRIFT_DETECTION_METHODOLOGY.md)
for the full statistics, or [§ Limitations](#limitations) for what this does
*not* catch before you rely on it.

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

The path segment is the **provider type** — `openai`, `anthropic`, or `azure` — and the body is just the secret: `{"key": "..."}`. The vault stores only the API key; model, base URL, api-version, and the Azure endpoint come from `[providers.*]` config, and the model/deployment is chosen per probe. Ollama needs no key (it has no auth), so it has no vault entry. A stored key takes effect on the next run — no restart.

### 3 — Start the daemon

```bash
modelsentry-daemon --config config/local.toml
# Listening on http://127.0.0.1:7740
```

### 4 — Create your first probe

Create a TOML file describing the probe:

```toml
# my_probe.toml
name     = "gpt-5-smoke"
schedule = { kind = "every_minutes", minutes = 60 }

# The provider spec fully declares the target. `kind` selects the provider and
# carries the model the probe actually runs:
#   { kind = "open_ai",   model = "gpt-5.4" }
#   { kind = "anthropic", model = "claude-sonnet-4-6" }
#   { kind = "ollama",    model = "llama3", base_url = "http://localhost:11434" }
#   { kind = "azure",     chat_deployment = "gpt-4o-prod", embedding_deployment = "embed-prod" }
provider = { kind = "open_ai", model = "gpt-5.4" }

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
Drift      None  (combined p = 0.41, target FPR 0.01)
Last run   2026-03-06 12:00 UTC (42 s)
Baseline   2026-03-06 11:45 UTC  (5 prompt clouds · 20 runs)
```

### 7 — Set up an alert

```bash
modelsentry alert create \
  --probe 018e1234-... \
  --target-fpr 0.01 \
  --webhook https://hooks.slack.com/services/...
```

When a run's calibrated combined p-value falls below `target_fpr`, the webhook
fires with a JSON payload containing the full `DriftReport`.

`target_fpr` is a **per-run** rate, so over many scheduled runs false alarms
accumulate. Two optional controls bound that: `[alerts] cooldown_secs` de-dups
repeat alerts for a rule (noise), and `[alerts.sequential]` (`window_secs`,
`alpha_budget`) applies **alpha-spending** to bound the *expected number of false
alarms per rule per rolling window* (error rate) — the formal sequential
guarantee. The latter is off by default; see
[methodology §8](docs/DRIFT_DETECTION_METHODOLOGY.md#8-sequential).

---

## How It Works

```
┌──────────────────────────────────────────────────────────────┐
│  Scheduler (tokio)                                           │
│  ┌──────────┐  run()  ┌──────────────┐  DriftReport         │
│  │  Probe   │ ──────► │ ProbeRunner  │ ──────────────────►  │
│  │ (config) │         │ (LLM calls)  │    DriftCalculator   │
│  └──────────┘         └──────────────┘    (conformal +      │
│                                            MMD/energy)       │
│                                               │              │
│                                         AlertEngine         │
│                                         (webhook / Slack)   │
└──────────────────────────────────────────────┴──────────────┘
         │ REST API (axum)                      │ redb storage
         ▼                                      ▼
   SvelteKit Dashboard                   .modelsentry/store.redb
```

Every run embeds the model's **completions** and compares the resulting
per-prompt output-embedding clouds against the baseline with a **calibrated
nonparametric two-sample test** (per-prompt conformal, with a pooled MMD/energy
permutation fallback). The result is a single calibrated **combined p-value**: a
run alerts when `combined_p_value < target_fpr`, so the threshold *is* your chosen
false-positive rate. See
[`docs/DRIFT_DETECTION_METHODOLOGY.md`](docs/DRIFT_DETECTION_METHODOLOGY.md).

The `DriftLevel` is derived from how many orders of magnitude the p-value falls
below your target FPR (α):

| Level | Condition |
|-------|-----------|
| `None` | `p ≥ α` (within normal noise) |
| `Low` | `α/10 ≤ p < α` |
| `Medium` | `α/100 ≤ p < α/10` |
| `High` | `α/1000 ≤ p < α/100` |
| `Critical` | `p < α/1000` |

Configure the single knob — and a richer baseline for more power — under
`[alerts]` (`target_fpr`, `baseline_capture_runs`, `cooldown_secs`, and the
optional `[alerts.sequential]` alpha-spending budget); see
[`docs/THRESHOLD_TUNING.md`](docs/THRESHOLD_TUNING.md).

---

## Example Probes

The Quickstart above uses trivial prompts to get you running fast. In
practice, a probe is only as good a canary as its prompts are sensitive to
the kind of regression you care about — a prompt with one defensible answer
that's cheap to phrase differently is a much better drift signal than one
with many equally-valid phrasings, since the embedding cloud is tighter and
a real shift stands out sooner. Precise, narrow-domain questions with a
single defensible current answer make good canaries for exactly that reason:
a version swap, a distillation, or a knowledge cutoff regression shows up as
a detectable shift, not noise. Five examples, with what a regression would
look like:

| Domain | Prompt | Why it drifts |
|---|---|---|
| CS — computability | "Is the equivalence problem for two arbitrary context-free grammars decidable?" | Correct answer is *no* (undecidable — unlike the regular-language case, which *is* decidable). A model that starts hedging into "it depends" or wrongly claims decidability is a real regression, not phrasing noise. |
| CS — complexity theory | "Is graph isomorphism known to be NP-complete, known to be in P, or open — and what's the best known worst-case algorithm?" | Correct answer names Babai's 2015 quasipolynomial algorithm. A model reverting to pre-2015 framing ("no subexponential algorithm known") signals a stale or downgraded model behind the same endpoint. |
| CS — approximation algorithms | "What's the best known polynomial-time approximation ratio for metric TSP, versus Christofides' classical 3/2 bound?" | Correct answer cites the Karlin–Klein–Oveis Gharan result (2020/2021), the first improvement on 3/2 in over 40 years. A stale answer just repeats "3/2, unimproved." |
| Mathematics — combinatorics | "What's the best known upper-bound exponent for the cap set problem, following the Croot–Lev–Pach / Ellenberg–Gijswijt polynomial method?" | Correct answer is the 2016 breakthrough bound (`O(2.756ⁿ)`, down from the trivial `~3ⁿ`). A model that can't name the polynomial method or the improved base is a checkable knowledge regression. |
| Astrophysics — cosmology | "What are the current SH0ES and Planck measurements of the Hubble constant H₀, and how large is the tension between them?" | Correct answer: ≈73 km/s/Mpc (SH0ES, local distance ladder) vs. ≈67 km/s/Mpc (Planck, CMB), tension around 5σ. An actively refined, precise, checkable pair of numbers — a good freshness canary in a fast-moving field. |

Each of these pairs naturally with `expected_contains` (e.g. `"Babai"`,
`"undecidable"`, `"Karlin"`, `"Ellenberg"`, `"73"`) so a hard factual
regression fails immediately, while the embedding-drift test still catches
subtler shifts in how the answer is phrased, hedged, or reasoned through.

---

## Limitations

- **Detects semantic-embedding drift on synthetic probes you configure —
  nothing more.** Format/JSON-validity breakage, latency regressions, tone
  shifts, refusals, and meaning-preserving safety regressions are currently
  invisible to it.
- **Synthetic canary probes are not production monitoring.** You only see
  drift on the prompts you configured, not on real traffic.
- **The calibrated test assumes baseline/run exchangeability.** Provider
  version changes and exact-request caching are now detected/mitigated
  (model-version pinning, cache-busting nonces); time-of-day effects and
  autocorrelation are not — treat drift near a deploy or traffic-pattern
  change with extra scrutiny. See
  [`docs/DRIFT_DETECTION_METHODOLOGY.md`](docs/DRIFT_DETECTION_METHODOLOGY.md)
  for the full statistical assumptions.

---

## Supported Providers

| Provider | Embeddings (→ drift) | Completions |
|----------|-----------|-------------|
| OpenAI | ✓ (`text-embedding-3-small` 1536d / `text-embedding-3-large` 3072d) | ✓ (`gpt-5.4`, `gpt-5.5`, etc.) |
| Azure OpenAI | ✓ (when an embedding deployment is configured) | ✓ (per-deployment) |
| Anthropic | — (no embedding API → completions-only, **no drift detection**) | ✓ (`claude-*`) |
| Ollama (local) | ✓ | ✓ |

Drift detection requires embeddings: providers without an embedding endpoint (or
Azure without an embedding deployment) run completions-only and never produce a
drift report. The probe fully declares its target (`provider` + model/deployment);
the daemon constructs exactly that provider per run from the encrypted vault (the
API key, keyed by provider type) plus `[providers.*]` config (base URLs,
api-version, the Azure resource endpoint).

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
    ├── ARCHITECTURE.md                  # System design + engineering standards
    ├── DRIFT_DETECTION_METHODOLOGY.md   # The statistics behind drift detection
    ├── THRESHOLD_TUNING.md              # Target-FPR tuning & baseline richness
    ├── LOCAL_CI_GUIDE.md                # Local hooks / CI-equivalent checks
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
npm run dev      # http://localhost:5173 — calls :7740 directly (CORS), no proxy
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
- Drift detection methodology (the statistics): `docs/DRIFT_DETECTION_METHODOLOGY.md`
- Target-FPR tuning & baseline richness: `docs/THRESHOLD_TUNING.md`
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
