# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog and this project adheres to Semantic Versioning.

## [Unreleased]

### Added

- **Sequential control — rolling-window alpha-spending.** New optional
  `[alerts.sequential]` block (`window_secs`, `alpha_budget`) bounds the
  **expected number of false alarms per rule per window**, the true sequential
  guarantee the per-rule cooldown could not give. `target_fpr` is a per-run
  rate, so an hourly probe at FPR 0.01 expects ~7 false alarms/month even with
  no drift; with this control each rule spends its testing level from a per-window
  budget on **every look** (debit-on-look), tested at
  `min(target_fpr, budget − spent)`, and is silenced once the rolling spend
  reaches `alpha_budget` until older spends age out. Spends are persisted to a
  new `alert_spend` redb table (pruned past the window) so the budget spans runs
  and restarts. Disabled by default (omit the block); orthogonal to and composes
  with `cooldown_secs`. (`crates/core/src/alert.rs` `SequentialControl`,
  `crates/store/src/spend_store.rs` `AlphaSpendStore`,
  `crates/daemon/src/scheduler.rs`)
- **Scheduler restart durability (catch-up).** Each probe's next-run time is now
  persisted (`schedule_state` redb table), so a restart resumes every probe's
  cadence instead of re-phasing it to "one interval from process start". A probe
  found overdue runs once immediately (single catch-up), then resumes normally.
- **Fleet-wide run concurrency cap.** `[scheduler] max_concurrent_runs` (default
  8) bounds probe runs executing at once across all probes via a shared
  semaphore, so a fleet — or a restart that finds several probes overdue — cannot
  stampede a provider.
- **Graceful shutdown.** The daemon now traps Ctrl+C / SIGTERM, drains in-flight
  HTTP requests, then stops the scheduler so probe loops abort cleanly (no
  half-written redb transaction on a hard kill).
- **Email alert channel (SMTP).** `AlertChannel::Email` now delivers over SMTP
  via `lettre` (rustls TLS) instead of logging a stub. Configure `[alerts.smtp]`
  (host, port, from, optional username, `security = start_tls | tls | none`); the
  password is read from the vault under the `smtp` key. The mailer is built once
  at startup — a misconfigured block disables email (logged) without aborting the
  daemon. (`crates/core/src/email.rs`, `crates/core/src/alert.rs`)
- **Alert cooldown / de-duplication (sequential control).** `[alerts]
  cooldown_secs` (default 3600) bounds repeat notifications: a rule that keeps
  firing within its window is de-duplicated — the run is still recorded, but no
  new notification/event is emitted. Closes the gap where the per-run `target_fpr`
  multiplied into many alerts over a month of scheduled runs.
- **Drift effect size (magnitude, not just significance).** `DriftReport` gains
  `effect_size` — how far the run's outputs moved, in standard deviations of the
  no-drift null — reported alongside the p-value and `−log₁₀(p)` score (which a
  large baseline can inflate for a trivial shift). Shown in the dashboard's
  *Latest Metrics* and in the verdict text.
- **Per-prompt multi-sampling removes the single-prompt drift floor.** Each run
  now samples every prompt `[alerts] samples_per_prompt` times (default 3), so a
  prompt's drift is scored by a two-sample energy permutation (baseline cloud vs
  run sample cluster) that resolves to `1/(B+1)` — even a *single* drifted prompt
  clears small target FPRs, no longer bounded by the `1/(k+1)` conformal rank.
  `samples_per_prompt = 1` keeps the cheaper single-sample (rank-limited) mode.
  `ProbeRun.embeddings` is now nested (per prompt → per sample → vector).
- **Azure OpenAI provider.** Full adapter (deployment-based URLs, `api-version`
  query param, `api-key` auth header, optional embedding deployment for drift).
  Configured via `[providers.azure]` (resource endpoint, api-version, embedding
  deployment) with the API key in the vault under `azure`.

### Changed

- **Constants consolidated into the single workspace module.** The drift-math
  floors/tolerances (previously redeclared in `twosample.rs` and `assessment.rs`)
  and the `[alerts]` default values now live in `modelsentry_common::constants`
  (`drift`, `alerts`, `credential` groups), so a tuning value is defined once.
- **Drift gate now resolves the target FPR (calibration fix).** The per-prompt
  conformal rank is hard-floored at `1/(k+1)`, so the previous Šidák-of-min gate
  could not alert at the default `target_fpr = 0.01` with `k = 20` baseline runs.
  The run-level gate is now a **stratified permutation test** of an aggregate
  statistic `T = Σ max(zᵢ, 0)` over standardized per-prompt excursions (exact
  under within-prompt exchangeability), with resolution `1/(B+1)`; permutations
  auto-raise so `1/(B+1) ≤ target_fpr`. Per-prompt conformal p-values are kept
  for attribution. Adds empirical null-FPR calibration and broad-drift power
  tests. (Single-prompt-only drift remains information-bounded at `1/(k+1)`.)
- **Unified provider subsystem.** `ProviderKind` → self-describing `ProviderSpec`
  (the model/deployment and any instance address live in the spec); the dead,
  silently-ignored top-level `Probe.model` field is removed. A single resolver
  (`provider_factory::build_provider`) constructs the exact provider a probe
  declares — secret from the vault (keyed by provider type), infrastructure from
  `[providers.*]` config — replacing the in-memory provider registry and the two
  duplicated `provider_key()` mappings. Providers are resolved per run, so a key
  added to the vault takes effect on the next run with no restart.
- **Vault API is store-only.** `PUT /api/vault/keys/{provider}` takes just
  `{"key": "..."}` (the `model`/`base_url` override fields are gone); keys are
  keyed by provider type (`openai`, `anthropic`, `azure`). Ollama needs no key.
- All compile-time constants (provider timeouts, upstream header names, HTTP
  body/rate limits, scheduler reconcile interval, run concurrency) are centralized
  in the `constants` modules rather than declared inline in feature files.
- **Drift detection rebuilt on a calibrated statistical foundation.** Runs now
  embed the model's **completions** (not the fixed prompts) and compare per-prompt
  output-embedding clouds to the baseline with a nonparametric two-sample test
  (per-prompt **conformal** p-values combined with the **Šidák** correction, and a
  pooled **MMD/energy** permutation-test fallback). A run alerts when its
  calibrated `combined_p_value < target_fpr`, so the alert threshold *is* the
  chosen false-positive rate. See `docs/DRIFT_DETECTION_METHODOLOGY.md`.
- `AlertRule` now uses a single `target_fpr` instead of `kl_threshold` /
  `cosine_threshold`; `[alerts]` config gains `target_fpr`, `baseline_capture_runs`,
  and `permutations`.
- `BaselineSnapshot` (schema v2) stores per-prompt output-embedding clouds
  aggregated over recent runs; `DriftReport` carries `combined_p_value`,
  `statistic`, `target_fpr`, `method`, a per-prompt breakdown, and an honest
  `interpretation`. Existing v1 baselines are detected and rejected with a
  re-capture message rather than silently misread.

### Removed

- Retired the discredited drift primitives — Gaussian `kl.rs`, `cosine.rs`,
  `entropy.rs`, and per-probe `adaptive.rs` thresholds — along with `PITCH.md`,
  whose framing was built on them. (`n ≪ d` embeddings make a Gaussian KL
  singular; the kernel/conformal methods are nonparametric and valid in that
  regime.)

## [0.1.0-init.0] - 2026-03-06

### Added

- Rust workspace scaffolding with five crates (`common`, `core`, `store`, `daemon`, `cli`)
- SvelteKit frontend scaffold under `web/`
- Shared workspace dependencies, lint policy, and toolchain setup
- Common crate Phase 1: newtype IDs, `ApiKey` with redacted serde, error hierarchy, domain models, TOML config with validation — 32 tests
- Enterprise repository governance (`LICENSE`, `SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, Dependabot)
- CI pipeline: Rust (check/fmt/clippy/test), security (cargo audit), frontend (check/build/audit)
- CodeQL static analysis and dependency review workflows
- Initial project architecture and phased execution plan docs
