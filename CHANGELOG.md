# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog and this project adheres to Semantic Versioning.

## [Unreleased]

### Added

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
