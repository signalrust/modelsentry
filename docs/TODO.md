# ModelSentry — Remaining Work

Status snapshot after the provider-unification + calibrated-drift rebuild
(`b5462d2`), per-prompt multi-sampling (`7beabae`), and the
email/cooldown/scheduler-persistence/effect-size/baseline-health batch (in the
working tree as of this audit — **verified green: `cargo fmt --check` clean,
`clippy -D warnings` clean, `cargo test --workspace` = 230 pass / 0 fail**).

**Priority legend.** **P1/Blocking** = a competent UI-only user cannot complete
the core loop (configure → probe → baseline → drift → alert) without it.
**P2** = correctness/quality that raises the product from "works" to "trustworthy."
**P3** = hygiene, docs, deploy polish.

> **Audit note (this pass).** Every "Done" item below was re-verified against the
> code, not just the changelog. Items that turned out to be already implemented
> were moved up from the TODO lists (baseline-health warning, f64 sums, Šidák
> scrub). The remaining items each carry file/line pointers and a quality note so
> the next session can act without re-discovery.

---

## Done (context — not TODO)

- **Unified provider subsystem** — self-describing `ProviderSpec`, single per-run
  resolver (`provider_factory::build_provider`), no registry, vault store-only.
  Frontend mirrors `ProviderSpec`; daemon is API-only; dashboard builds with
  `adapter-node`.
- **Azure OpenAI** provider (adapter + `[providers.azure]` config + UI). *Now on
  the **v1 API** (GA since Aug 2025): versionless `/openai/v1/{op}` path with the
  single stable `api-version=preview` channel (no dated monthly versions), and the
  deployment passed as the body `model` field. `defaults::azure::API_VERSION =
  "preview"`.*
- **Provider model constants refreshed to June 2026** (`constants.rs` `defaults`):
  per-provider `MODELS` top-3 menus + a derived default `MODEL` — OpenAI
  `gpt-5.5`/`gpt-5.4`/`gpt-5.4-mini`, Anthropic `claude-opus-4-8`/
  `claude-sonnet-4-6`/`claude-haiku-4-5`, Ollama `llama4`/`qwen3`/`gemma4`, Azure
  reuses OpenAI's. Anthropic `API_VERSION` stays `2023-06-01` (the live contract
  identifier, not a stale date).
- **Drift detection rebuilt** on a calibrated two-sample foundation (schema v2):
  measures *completions* not prompts; conformal per-prompt + MMD/energy +
  permutation; honest interpretation layer.
- **Calibration fix (was the flagship bug).** The old Šidák-of-min gate inherited
  the per-prompt conformal floor `1/(k+1)`, so at defaults (k=20, fpr=0.01) the
  preferred detector mathematically could never fire. Replaced with a **stratified
  permutation gate** on `T = Σ max(zᵢ,0)` (standardized per-prompt excursions):
  resolves to `1/(B+1)`, needs no independence assumption, uses magnitude not just
  rank, and auto-raises `B` so `1/(B+1) ≤ target_fpr` (never silently un-fireable).
  Per-prompt conformal p-values retained for **attribution**. Variance floor for
  near-deterministic baselines. Regression guard added: empirical null-FPR
  Monte-Carlo test + broad-drift-below-the-floor power test.
  (`crates/core/src/drift/assessment.rs`)
- **Sample each prompt N times per run** — `[alerts] samples_per_prompt`
  (default 3); each prompt scored by a two-sample energy permutation, removing the
  single-prompt `1/(k+1)` floor even for single-prompt drift.
- All compile-time constants centralized in the single workspace `constants`
  module (provider/vault keys, method tags, tables, headers, provider defaults,
  drift floors, alert defaults) + the frontend `constants.ts`.
- **Email alert channel (SMTP).** `AlertChannel::Email` delivers over SMTP via
  `lettre` (rustls); `[alerts.smtp]` config + vault-held password; mailer built
  once at startup, misconfig disables email without aborting.
  (`crates/core/src/email.rs` — `EmailMailer`, TLS/STARTTLS/plaintext, 4 tests.)
- **Sequential control — rolling-window alpha-spending.** *(P2 statistical rigor
  — the headline calibration remainder, now shipped.)* Optional
  `[alerts.sequential]` (`window_secs`, `alpha_budget`) bounds the **expected
  number of false alarms per rule per window** — the guarantee the per-rule
  cooldown could not give. Each look spends `min(target_fpr, budget − spent)`
  from the window budget (debit-on-look, so `Σ levels = E[false alarms] ≤
  alpha_budget`); the rule is silenced once exhausted until spends age out.
  Spends persist in a new `alert_spend` redb table (pruned past the window), so
  the budget spans runs and restarts. Disabled by default; composes with
  cooldown. (`crates/core/src/alert.rs` `SequentialControl`/`AlertOutcome`,
  `crates/store/src/spend_store.rs` `AlphaSpendStore`, wired in `main.rs` +
  `scheduler.rs`; methodology §11.) Residual is dashboard-only (P2-UX).
- **Alert cooldown / de-duplication.** `[alerts] cooldown_secs` (default 3600)
  de-dups repeat notifications per rule (run still recorded). Engine takes a
  store-loaded last-fired map; the per-run vs per-period distinction is documented.
  (`crates/core/src/alert.rs:64-140`, `with_cooldown` / `in_cooldown`, both
  directions tested.)
- **Drift effect size (magnitude).** `DriftReport.effect_size` — drift magnitude
  in null SDs (mean standardized excursion), separating effect size from
  `−log₁₀(p)` precision; flows model → `types.ts` → dashboard ("Magnitude … SD"
  in `DriftMetrics.svelte`) and the verdict text.
- **Baseline-health warning.** *(was P2 — verified done this audit.)* Per prompt,
  `PromptDrift.low_variance_baseline` flags a near-constant baseline cloud
  (`cloud_spread < BASELINE_MIN_CLOUD_SPREAD`, `assessment.rs:214`/`:484`). It
  flows into the API model (`models.rs PromptDrift`) and is surfaced to the
  operator as a "⚠ Baseline health" sentence in the interpretation text
  (`interpret.rs:30-38`), which the dashboard renders (`DriftMetrics.svelte:37`).
  *Residual polish (now tracked under P2-UX):* the frontend `PromptDrift`
  interface (`types.ts:59-63`) does not carry the boolean, so there is no
  structured per-prompt badge — only the prose warning.
- **`f64` for the kernel/statistic sums.** *(was P2 — verified done this audit.)*
  Every accumulation path is `f64`, cast to `f32` only at the public boundary:
  `twosample.rs` `sq_dist`/`gram_matrix`/`statistic_from_gram`/
  `standardized_excursion`; `assessment.rs` `mean_std`/`euclidean`/`cloud_spread`.
- **Scheduler next-run persistence + catch-up.** Per-probe next-run is persisted
  (`schedule_state` table, `crates/store/src/schedule_store.rs`); on restart an
  overdue probe runs once (catch-up) then resumes its cadence
  (`scheduler.rs:286`/`:315`, test `overdue_probe_runs_immediately…`).
- **Global concurrency cap.** `[scheduler] max_concurrent_runs` (default 8) bounds
  concurrent runs fleet-wide via a shared `Semaphore` (`scheduler.rs:45`/`:261`).
- **Graceful shutdown.** Ctrl+C / SIGTERM drains the HTTP server
  (`server.rs:177` `with_graceful_shutdown`) then stops the scheduler
  (`main.rs:196`).
- **Stale Šidák references scrubbed** from the code-describing docs. *(was P3.)*
  No doc describes the *current* gate as Šidák — ARCHITECTURE's module/test notes
  now say "stratified permutation gate". The methodology **intentionally** retains
  Šidák in its "why we replaced it" discussion, glossary, and citations; those are
  historical/reference, not a description of current behavior.
- Docs refreshed (README, ARCHITECTURE, methodology §8, CHANGELOG, config).
- Verification this audit: `clippy -D warnings` clean; `cargo fmt --check` clean;
  `cargo test --workspace` **230 pass / 0 fail** (core 114 incl. 10 integration,
  daemon 50, store 25, common 41, cli 0).

---

## P1 — Core dashboard loop (UI-only users are blocked)

The API + CLI exist for all of these; the dashboard only *displays* state. **None
of the four are wired** (confirmed: the API methods below are defined but called
nowhere under `web/src/routes` or `web/src/lib/components`).

- [ ] **Baseline capture button.** `api.baselines.captureForProbe`
      (`web/src/lib/api.ts:236`) exists but is never called. Add a capture action
      on the probe detail page with loading / error / "needs N runs" states;
      refresh baseline + drift after.
      (`web/src/routes/probes/[id]/+page.svelte`)
- [ ] **Alert-rule creation.** `api.alerts.createRuleForProbe` (`api.ts:255`),
      `listRulesForProbe` (`:252`), and `deleteRule` (`:261`) all exist but are
      never called. Add a form (target FPR + channel: webhook / Slack / email),
      list existing rules, add a delete action.
- [ ] **Anthropic "completions-only, no drift" disclaimer in the new-probe form.**
      README documents it; Azure already shows an embedding hint
      (`AddProbeForm.svelte:149-155`) — add the *parity* warning when **Anthropic**
      is selected. Currently there is no Anthropic-specific notice.
      (`web/src/lib/components/AddProbeForm.svelte`)
- [ ] **First-run onboarding panel.** Today there is only a bare "No probes yet —"
      empty state (`web/src/routes/probes/+page.svelte:72`); no real checklist.
      Add a lightweight checklist when there are no probes / no API key (vault
      passphrase, provider key, dashboard API key). Provider keys are CLI/API-only
      today — this is the path to surface them.

## P2 — Statistical rigor (path to a 9–10 calibration story)

**Exchangeability hardening — NEXT PRIORITIES.** Conformal/permutation validity
assumes baseline/run exchangeability; the two biggest *silent* breakers are a
model-version change and provider-side caching. Turn them from silent confounders
into detected/closed:

- [x] **Model-version pin + detection — DONE.** Each provider's `complete` now
      returns a `Completion { text, model_version }` parsed from the response
      `model` field (OpenAI/Azure/Ollama `model`, Anthropic `model`). The run
      records the observed version (`ProbeRun.model_version`); baseline capture
      pins it (`BaselineSnapshot.model_version`); `DriftCalculator::compute`
      compares them and sets `DriftReport.model_version_changed`, with the
      `before → after` named in the verdict text (`interpret.rs`) — only flagged
      when *both* versions are known (an unreported version is never guessed).
      (`crates/core/src/provider/*`, `probe_runner.rs`, `drift/calculator.rs`,
      `drift/interpret.rs`, `routes/baselines.rs`, `common/models.rs`.)
- [x] **Cache-busting — DONE.** Each cloud-provider completion request carries a
      fresh non-semantic nonce (`provider::cache_bust_nonce`, a per-call UUID):
      OpenAI/Azure send `store: false` + a varying `user`; Anthropic sends
      `metadata.user_id`. This defeats an exact-request cache that could return
      identical completions and collapse a baseline cloud to a near-constant
      (faking determinism). Ollama is exempt (local, no shared cache). The model
      never sees the nonce, so probe semantics are unchanged.
- [ ] **Time-of-day effects — DOC ONLY, do not engineer.** Proper handling means
      stratifying baselines by time window (splits data → less power) or schedule
      jitter (weak), for a confounder of unproven magnitude in LLM APIs. Keep it as
      a stated caveat (see Honesty section), not code.
- [ ] **Sequential control — dashboard surfacing (the UI remainder).** The
      **backend alpha-spending control is shipped** (see Done): `[alerts.sequential]`
      (`window_secs`, `alpha_budget`) bounds the expected false alarms per rule per
      rolling window via debit-on-look, persisted in the `alert_spend` table. What
      remains is **dashboard-only**: surface the per-run vs per-period distinction
      and show each rule's remaining budget / spend ("0.03 of 0.05 spent this
      window"). No further backend work needed for the guarantee itself.
- [ ] **Effect-size *direction* (optional follow-up).** The **magnitude** half
      (`DriftReport.effect_size`, in null SDs) is **done** (see Done). Remaining is
      only the optional extra: report a *direction* in embedding space, not just
      magnitude. Low priority — magnitude already answers "how big."

## P2 — UX quality

- [ ] **Live data refresh / honest LIVE badge.** The badge is a static decorative
      element (`+layout.svelte:66-69`) and every page loads once via
      `onMount(load)` with no polling (`+page.svelte:103`, `probes/+page.svelte:35`,
      `probes/[id]/+page.svelte:98`; the only `onDestroy` clears a toast timer, not
      a poller). Either add polling (interval + `onDestroy` cleanup + pause on
      hidden tab / in-flight run) or make the badge reflect real state.
- [ ] **Baseline-health: structured per-prompt badge.** *(Carved out of the now-
      done warning.)* The operator is already warned in prose
      (`interpret.rs` → `DriftMetrics.svelte`), but `types.ts:59-63 PromptDrift`
      omits `low_variance_baseline`, so there is no structured per-prompt badge.
      Add the field to the TS interface (it's already in the Rust model + JSON) and
      render a small "noisy baseline" chip per prompt. Polish, not blocking.
- [ ] **CSS: 100% from theme tokens.** Hardcoded colors remain: ~15 matches in
      `web/src/app.css` and 1 in `ApiKeyDialog.svelte` (`#fff`, `rgba(0,0,0,…)`).
      Route them through per-theme tokens: `--on-accent` (`#fff` literals),
      `--scrim` (sidebar overlay + dialog backdrop), soft fills
      (`--fill-up/warn/down/info`), `--focus-ring`, `--card-sheen`,
      `--btn-primary-shadow`, pulse-keyframe colors. Verify `DriftChart` reads
      chart colors via `getComputedStyle` (re-themes).
- [ ] **Accessibility.** `ApiKeyDialog` Escape only fires when the input has focus
      (handler is on the input, `ApiKeyDialog.svelte:25`/`:58`) — make Escape close
      regardless of focus, add a focus trap and focus restore on close. Mobile
      sidebar overlay (`+layout.svelte` `closeDrawer`) has no keyboard dismissal.
- [ ] **`run.status` display helper.** Replace the inline `replace('_', ' ')` at
      `ProbeTable.svelte:104`, `+page.svelte:53`, and `probes/[id]/+page.svelte:192`
      with a typed helper keyed off the status constants. (Note: the
      `provider.kind.replaceAll('_',' ')` fallbacks are a separate concern.)

## P3 — Hygiene / docs / deploy

- [x] **Gate null-pool doc overclaim — DONE.** `stratified_permutation_p`'s doc
      now states the null pool is path-dependent (≥2-sample = permutation null,
      **excludes** the observed point; 1-sample conformal = augmented scores,
      **includes** it), that strata are therefore heterogeneous, and that the
      `1/(n_perm+1)` floor comes from the `+1` in the estimate, not from including
      the observed point. (`assessment.rs`)
- [x] **CORS `X-Api-Key` — DONE.** `server.rs` CORS `allow_headers` now includes
      `app_header::API_KEY` alongside `CONTENT_TYPE`/`AUTHORIZATION`, so a browser
      client on a non-`*` origin clears preflight with either auth scheme.
- [ ] **README "proxies /api" fix.** Still wrong: `README.md:258` says
      `npm run dev … (proxies /api to :7740)`. The dev server does **not** proxy;
      the browser calls `:7740` directly (CORS). Correct the wording.
- [ ] **Release checklist + test counts.** Reconcile
      `docs/RELEASE_READINESS_CHECKLIST.md` and any stale counts before tagging —
      the current suite is **230** tests (this doc previously said 200).
- [ ] **Track `proc-macro-error2` (unmaintained).** Transitive via `tabled`/`age`;
      not a CVE, allowed by `cargo audit`. Revisit on upstream upgrades.
- [ ] **No-magic-values sweep (frontend).** Audit remaining inline literals in
      `web/src` (status strings, storage-event names, toast durations, JS
      breakpoints) and route them through `constants.ts`. Same for any drift
      weights/thresholds still inline in `crates/core/src/drift`.

---

## Known limitations & architecture smells (backlog, non-blocking)

- **Run / event retention.** `RunStore`/`AlertRuleStore` still grow unbounded —
  every scheduled run and event is stored forever. Add a retention/pruning policy
  before any long-running deployment. *(The read-side scan smell is now fixed for
  runs — see "Resolved"; this is the remaining storage-growth half.)*
- **`AlertRuleStore::list_events` still full-scans.** It iterates and
  deserializes every event to return the recent N. Only hit by the manual events
  route (not the scheduler), so low priority — a time-ordered index keyed like
  `run_index` would make it `O(limit)` and pair naturally with event retention.
  (`last_fired_for_rule` is now O(1) — see "Resolved".)
- **Blocking redb I/O on async worker threads.** Store calls are synchronous and
  run inside async tasks. *Re-scoped after the run-storage split + cooldown
  index:* the pathological scans are gone, so store ops on the scheduler path are
  now µs-scale point lookups / bounded range scans where `spawn_blocking`'s
  thread-handoff would cost more than it saves. Revisit only if profiling shows a
  hot spot (e.g. the CPU-bound drift permutation test → `block_in_place`); not a
  blanket wrap.
- ~~**`ModelSentryError::Provider { message }` is a stringly-typed catch-all.**~~
  **DONE.** Split into matchable variants: `ProviderTransport` (network/transport
  failure), `ProviderDecode` (response-body parse failure), `Drift` (drift-data
  validation: prompt-count mismatch / no usable prompt / too few samples /
  non-finite embedding), and `Internal` (e.g. "semaphore closed"). `Provider` now
  carries only genuine *semantic* provider failures (no choices, safety refusal,
  unsupported capability). All provider/drift/runner sites + `# Errors` docs
  updated; AppError still maps the non-semantic ones to 500 via its catch-all.
- **`let out = (…); out` attribute-scoping workaround.** Used only to attach an
  `#[allow(clippy::cast_*)]` to an expression (e.g. `assessment.rs:301`,
  `euclidean`, `twosample.rs next_bounded`). A module-level `#![allow]` (as
  `assessment.rs` already does for `cast_precision_loss`) would remove the
  boilerplate. (Trivial.)
- ~~**`scheduled_next_run` error branch doesn't persist**~~ — **DONE.** The
  store-read-error branch now persists the fallback next-run (via
  `advance_schedule_to`, like the fresh-probe branch), so a transient read error
  no longer re-phases the probe every tick; restart precision resumes once the
  store recovers. (`scheduler.rs`)
- **Test gaps (qualitative).** No frontend tests. Calibration is empirically
  validated (null-FPR Monte-Carlo) and the scheduler has restart-catch-up and
  shutdown tests, but there are still no store concurrency-stress tests.

### Resolved (was here)

- ~~Per-run `reqwest::Client` rebuild~~ — **done.** Providers take an injected
  process-wide pooled client; per-request timeouts preserve per-provider values.
- ~~Embeddings sent one-per-call (batch size 1)~~ — **done.** The runner batches a
  prompt's samples into a single `embed` request.
- ~~Redundant full Gram matrix~~ — **done.** Only the upper triangle is computed
  and mirrored (symmetric kernel).
- ~~`last_fired_for_rule` full-scans every event (scheduler hot path)~~ —
  **done.** A per-rule `alert_last_fired` index (maintained on insert, cleared on
  rule delete) makes the cooldown lookup O(1). This is the substance of the
  "blocking redb I/O" concern — eliminated the slow op rather than wrapping it.
- ~~Run listing full-scans + decodes all embeddings~~ — **done.** Runs are split
  into metadata / embeddings / time-ordered index tables; `list_for_probe` is a
  bounded `O(limit)` range scan that never decodes embeddings, and embeddings are
  fetched (by run id) only by baseline capture. (`run_store.rs`)
- ~~Pooled drift fallback silently un-fireable below the permutation floor~~ —
  **done.** The pooled path now applies the same `min_perms_for_resolution`
  guard as the per-prompt gate. (`assessment.rs`)
- ~~Scheduler does not persist next-run times~~ — **done.** Per-probe next-run is
  persisted (`schedule_state` table); on restart an overdue probe runs once
  (catch-up) then resumes its cadence.
- ~~No global concurrency cap across probes~~ — **done.** `[scheduler]
  max_concurrent_runs` (default 8) bounds concurrent runs fleet-wide via a shared
  semaphore.
- ~~No graceful shutdown~~ — **done.** Ctrl+C / SIGTERM drains the HTTP server then
  stops the scheduler.
- ~~Baseline-health warning~~ — **done** (P2). Surfaced in the verdict text;
  residual structured-badge polish moved to P2-UX.
- ~~f64 kernel/statistic sums~~ — **done** (P2).
- ~~Stale Šidák references~~ — **done** (P3); no doc describes the *current* gate
  as Šidák (methodology keeps it only as historical/citation context).

## ⭐ Premium / vision — broaden what "drift" means (post-v1, additive)

These close the *claims-vs-reality* gap from the **Honesty** section below by
actually doing more, not just rewording. **Crucially, this is additive, not a
rewrite:** the calibrated engine (conformal + MMD/energy + permutation +
sequential control), storage, scheduler, and alerting all stay — these feed new
signals/traffic into the *same* `DriftReport`/alert pipeline. (~70% of existing
work is reused.) Recommended order is **②, then ③, then ①** (cheap-and-broadening
first, the differentiator last).

- [ ] **⭐ ② Multi-signal detectors (do first — cheap, biggest credibility jump).**
      Embedding shift is one axis; "drift monitoring" only becomes *true* with
      more. Add small, pluggable detectors that each emit into the existing
      severity/alert path (~a day each): output **format/schema validity**,
      **refusal rate**, **latency**, **output-length distribution**,
      **sentiment/tone**. No engine change — each is a new signal source feeding
      the same report.
- [ ] **⭐ ③ Structured expectation/assertion checks.** Extend the probe's existing
      `expected_contains` to structured per-prompt assertions (JSON shape, regex,
      value bounds). Small; turns probes into contract tests as well as drift
      canaries.
- [ ] **⭐ ① Production-traffic sampling (the real differentiator — biggest build).**
      Today drift is only seen on a handful of synthetic probes. Mirror/sample
      **real requests** and run the same drift test on them. This is the one
      genuinely new subsystem — an ingestion path + PII/privacy handling — and it
      is what converts "synthetic canary" into actual *monitoring*. Still plugs
      into the existing pipeline; biggest effort, highest payoff.

> These are a **premium tier**, gated behind the v1 honesty fixes (Honesty
> section) and the NEXT exchangeability-hardening items (P2). Do not start them
> before the core dashboard loop (P1) is usable.

---

## Honesty / scoping (fix the *copy*, not just the code)

These are positioning corrections a referee/buyer would catch — keep claims tight:

- **It detects semantic-embedding drift on synthetic probes — say exactly that.**
  Format/JSON-validity breakage, latency, tone, refusals (partial), and
  meaning-preserving safety regressions are invisible to it.
- **Synthetic canary probes ≠ production monitoring.** You only see drift that
  hits your handful of prompts; competitors watch real traffic. State it.
- **Conformal validity assumes baseline/run exchangeability** — provider version
  pinning, time-of-day, caching, and autocorrelation can break it. Note as a caveat.
- **The sharp edge is self-hosted / nothing-leaves-the-box** (regulated/private
  deployments) — lead with that, not "more statistics."

---

## Audit scorecard (latest)

| Dimension | Score | Note |
|---|---|---|
| Statistical method (design) | 8/10 | Correct conformal + MMD/energy + permutation; right tools for n≪d. |
| Statistical method (calibration) | 9/10 | Stratified-permutation gate + per-prompt multi-sampling; empirically calibrated; f64 sums + baseline-health done; **alpha-spending / sequential control now shipped** (`[alerts.sequential]`, bounds expected false alarms per rule per window). Residual to 10: dashboard surfacing of the budget (UX, not method). |
| Architecture / design | 8/10 | Unified `ProviderSpec`, single resolver, clean layering. |
| Code quality | 9/10 | `clippy -D warnings`, no-unwrap lint, centralized constants, real docs, f64 numeric care. |
| Test quality | 7/10 | Validates calibration (null-FPR MC) + power; 230 tests; ~no frontend tests. |
| Frontend | 5/10 | Good design system; **control plane still incomplete (P1)**; static LIVE badge; theme leaks. |
| Security hygiene | 8/10 | age vault, constant-time key compare, SSRF guard, body/rate limits. |
| Product completeness | 4/10 | Core loop still needs CLI for baseline/rule capture; onboarding friction. (Email + cooldown shipped.) |
| Honesty (claims vs reality) | 4/10 | Overclaims above; scope copy needs tightening. |

**Resolved since the audits:** the "default detector is mute" flagship bug, the
"add an empirical calibration test," and the single-prompt `1/(k+1)` floor (via
multi-sampling) are all done and proven. The Šidák-independence critique is moot —
that gate was replaced.

**Path to a credible v1 (the audit's blocking set, current state):**
1. ~~Fix default sensitivity / un-fireable detector~~ — **done.**
2. ~~Empirical calibration test~~ — **done.** (Sequential/alpha-spending **now
   also done** — `[alerts.sequential]`; only the dashboard surfacing of the
   budget remains → P2-UX.)
3. Honest scoping in copy → Honesty section above.
4. **Complete the dashboard loop (baseline capture + alert rules) and live refresh
   → P1 above.** This is now the single biggest gap to a UI-usable v1.

---

## Notes

- Greenfield: no production data, so v1→v2 baseline migration UX is **not** needed
  (old baselines are simply re-captured).
- **Build env:** `cargo` is not on the default PATH in this workspace; it lives at
  `C:\Users\notk\.cargo\bin` (cargo/rustc 1.96.0). The git pre-commit hook runs
  `cargo fmt --check` + `clippy -D warnings` (+ `svelte-check` when `web/` is
  staged), so commits require that on PATH.

  ## Last audit

  6. model_version "first wins" hides within-run disagreement (minor — I added this)
probe_runner.rs records the first reported version across samples/prompts. If a provider serves a partial rollout (different versions mid-run), the discrepancy is silently dropped — ironic for a model-version detector. A tracing::debug! (or warn) when a later sample reports a different version would close it.

Magic strings
7. Provider/output wire literals (low — wire-contract constants, arguably acceptable inline but technically magic)

Role "user": openai.rs:234, azure.rs:286, anthropic.rs:162
Anthropic block_type == "text" / stop_reason == "refusal" / "end_turn": anthropic.rs:200-210
Schemes "http"/"https": alert.rs:322
Webhook payload keys "event_id"/"rule_id"/"drift_level"/"fired_at": alert.rs:255-259 — this is your outbound contract; worth a typed payload struct so it can't silently drift.
8. Test-fixture duplication (low)
timeout_secs: 30 plus the full AppConfig/AppState literal is copy-pasted across 5 route test modules (runs/probes/alerts/baselines/vault). A shared test_state() helper would remove ~5 duplicates and the magic 30.

Worth verifying (not clearly wrong)
9. SSRF allowlist edge ranges — is_disallowed_ip covers loopback/RFC-1918/link-local/metadata well, but not 100.64.0.0/10 (CGNAT) or 198.18.0.0/15 (benchmark). Realistic targets are covered; these are completeness gaps.

10. Cache-busting field efficacy (I added this session) — the varying user field defeats exact-request/proxy caches and store:false prevents storage, but user does not key OpenAI's prompt cache (that's the prompt prefix / prompt_cache_key). For short probe prompts prompt-caching doesn't apply and sampled outputs still vary, so it's defensible — but if defeating prompt caching is the explicit goal, prompt_cache_key is the precise knob. Worth confirming against the current API.

//end last audit
