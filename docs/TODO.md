# ModelSentry — Remaining Work

**~80% to a UI-usable v1.** The statistical engine, providers, alerting, and
storage are done and tested. What's left is concentrated almost entirely in
the dashboard control-plane wiring (P1, 0% done) plus a smaller set of
correctness items (P2) and hygiene (P3).

**Verified as of commit `7ecbadf`:** `cargo fmt --check` clean, `clippy -D
warnings` clean, `cargo test --workspace` = **245 pass / 0 fail** (common 42,
core 118, core-integration 10, daemon 50, store 25, cli 0).

---

## Done

- Unified provider subsystem — `ProviderSpec`, single per-run resolver, vault-only keys.
- Azure OpenAI provider, on the v1 API (versionless path, `api-version=preview`).
- Provider model menus refreshed (OpenAI/Anthropic/Ollama/Azure defaults).
- Drift detection rebuilt on a calibrated two-sample foundation (schema v2): completions not prompts, conformal per-prompt + MMD/energy + permutation.
- Calibration fix (flagship bug): un-fireable Šidák-of-min gate replaced with a stratified permutation gate; empirical null-FPR + power regression tests added.
- Per-prompt multi-sampling (`samples_per_prompt`), removing the single-prompt floor.
- All compile-time constants centralized (workspace `constants` module + frontend `constants.ts`).
- Email alert channel (SMTP via `lettre`, vault-held password).
- Sequential control / alpha-spending (backend) — bounds expected false alarms per rule per rolling window, persisted across restarts.
- Alert cooldown / de-duplication per rule.
- Drift effect size (magnitude, in null SDs) — model → API → dashboard.
- Baseline-health warning (near-constant baseline detection) — surfaced in verdict prose.
- `f64` precision throughout kernel/statistic accumulation.
- Scheduler: next-run persistence, restart catch-up, global concurrency cap, graceful shutdown, and fallback next-run now persists on a transient store-read error.
- Model-version pin + detection — flags a silent model swap between baseline and run.
- Cache-busting nonces on every cloud-provider completion request.
- Typed provider errors — `Provider` split into `ProviderTransport` / `ProviderDecode` / `Drift` / `Internal`.
- CORS `allow_headers` includes `X-Api-Key`.
- Perf: shared HTTP client, batched embeddings, half-size Gram matrix, split run storage, O(1) cooldown index.
- Docs refreshed (README, ARCHITECTURE, methodology, CHANGELOG, config).
- Build toolchain fixed: global `.cargo/config.toml` pins the MSVC linker by absolute path (Git's own `link.exe` was shadowing it) — plain `cargo build`/`cargo test` now work with no `vcvars64.bat` step.
- Honest scoping — README gained a `## Limitations` section (synthetic-probe-only scope, not production monitoring, exchangeability caveat); ARCHITECTURE.md and DRIFT_DETECTION_METHODOLOGY.md each cross-link it with a short scope callout.
- DRIFT_DETECTION_METHODOLOGY.md rewritten (630 → 270 lines): dropped the beginner glossary/tutorial framing for a dense, citation-anchored register (added a §5 Effect size section and a Lan–DeMets 1983 citation for the alpha-spending adaptation, both previously undocumented); every claim re-verified against `twosample.rs`/`assessment.rs`/`alert.rs`/`spend_store.rs`/config defaults. Cross-doc `#11-sequential` anchor links (README, THRESHOLD_TUNING.md) fixed to the new `#8-sequential`.
- README "proxies /api" fix — corrected to state the dev server calls `:7740` directly via CORS, no proxy.
- README rewritten for a technical audience (concrete problem statement up front, no marketing language) and gained an `## Example Probes` section with 5 worked examples (3 CS, 1 math, 1 astrophysics) demonstrating what a good drift-canary prompt looks like.

---

## Next To Do

### Blocking — a UI-only user cannot complete the core loop without these

The API and CLI already support all four of these end to end. **None of the
four are wired into the dashboard** — the API client methods exist and are
called nowhere under `web/src/routes` or `web/src/lib/components`. This is
the single biggest gap to a usable v1.

1. **Baseline capture button.**
   `api.baselines.captureForProbe` (`web/src/lib/api.ts:236`) exists but is
   never called. Add a capture action to the probe detail page
   (`web/src/routes/probes/[id]/+page.svelte`): a button that calls it, with
   loading / error / success states, and a "needs N runs before capture"
   state if the API returns that condition. After a successful capture,
   refresh the baseline and re-render the drift panel so the new baseline is
   reflected immediately rather than requiring a page reload.

2. **Alert-rule creation, listing, and deletion.**
   `api.alerts.createRuleForProbe` (`api.ts:255`), `listRulesForProbe`
   (`api.ts:252`), and `deleteRule` (`api.ts:261`) all exist but are never
   called from any component. Build a form on the probe detail page with:
   target FPR (numeric input, e.g. 0.01–0.10), and channel selection
   (webhook URL / Slack webhook URL / email address — the three variants
   `AlertChannel` already supports per `types.ts`). Below the form, list the
   probe's existing rules (target FPR + channel + active flag) with a delete
   button per rule that calls `deleteRule` and removes it from the list on
   success.

3. **Anthropic "completions-only, no drift" disclaimer.**
   Azure already shows an embedding-deployment hint when selected
   (`AddProbeForm.svelte:147-157`) because Anthropic has no embeddings API,
   so a probe against Anthropic can run completions but cannot compute drift
   — this is documented in the README but has no UI equivalent. Add a notice
   block in `AddProbeForm.svelte`, shown when `providerKind ===
   PROVIDER_KIND.ANTHROPIC`, stating plainly that drift detection will not
   run for this probe (completions-only monitoring).

4. **First-run onboarding checklist.**
   The empty state today (`web/src/routes/probes/+page.svelte:70-76`) is a
   single "No probes yet —" line with a "+ New Probe" button — no guidance
   for what has to be true before a probe can actually work. Add a
   lightweight checklist shown when there are no probes and/or no dashboard
   API key set: (a) vault passphrase configured server-side, (b) at least
   one provider API key stored via the vault endpoint (`PUT
   /api/vault/keys/{provider}` — this is CLI/API-only today, so the
   checklist is also the natural place to surface that this step exists and
   how to do it), (c) dashboard API key entered (already handled by
   `ApiKeyDialog`). Each item shows done/pending state if the API exposes
   enough to check it; otherwise show it as a static instruction.

### Critical — calibration story remainder

5. **Sequential control — dashboard surfacing.**
   The backend guarantee is fully shipped (`[alerts.sequential]`
   `window_secs`/`alpha_budget`, debit-on-look, persisted in the
   `alert_spend` table — see Done). What's missing is purely visual: on the
   probe/rule view, show each rule's current window spend against its budget
   (e.g. "0.03 of 0.05 spent this window") and make clear that `target_fpr`
   is a **per-run** rate while the sequential control bounds a **per-window**
   expected-false-alarm rate — these are different guarantees and the UI
   should not conflate them. No backend work needed.

6. **Time-of-day effects — stays a documented caveat, not code.**
   Proper handling would mean stratifying baselines by time window (splits
   the data, reduces statistical power) or schedule jitter (weak effect) —
   not worth building for a confounder of unproven magnitude in LLM APIs.
   Already listed as a caveat in the README/METHODOLOGY scope notes (see
   Done) — do not implement it.

7. **Effect-size direction (optional, low priority).**
   Magnitude (`DriftReport.effect_size`, in null SDs) already ships. The
   optional remainder is reporting a *direction* in embedding space in
   addition to magnitude. Low priority — magnitude alone already answers
   "how big was the drift."

### Everything else — UX polish, hygiene, and backlog

8. **Live data refresh / honest LIVE badge.**
   The "LIVE" badge (`+layout.svelte:66-69`) is static decoration; every
   page loads once via `onMount(load)` with no polling
   (`+page.svelte:103`, `probes/+page.svelte:35`,
   `probes/[id]/+page.svelte:98` — the only existing `onDestroy` clears a
   toast timer, not a poller). Either add real polling (interval +
   `onDestroy` cleanup + pause when the tab is hidden or a run is in
   flight) or remove the badge so it stops implying live state that doesn't
   exist.

9. **Baseline-health structured per-prompt badge.**
   The prose warning already renders (`interpret.rs` → `DriftMetrics.svelte`),
   but `types.ts:59-63` `PromptDrift` doesn't carry
   `low_variance_baseline`, even though it's already in the Rust model and
   the JSON response. Add the field to the TS interface and render a small
   "noisy baseline" chip per affected prompt.

10. **CSS: route remaining hardcoded colors through theme tokens.**
    ~15 matches in `web/src/app.css` and 1 in `ApiKeyDialog.svelte`
    (`#fff`, `rgba(0,0,0,…)`). Introduce/route through `--on-accent`,
    `--scrim` (sidebar overlay + dialog backdrop), soft fills
    (`--fill-up/warn/down/info`), `--focus-ring`, `--card-sheen`,
    `--btn-primary-shadow`, and the pulse-keyframe colors. Confirm
    `DriftChart` still re-themes correctly (it reads chart colors via
    `getComputedStyle`).

11. **Accessibility.**
    `ApiKeyDialog`'s Escape handler only fires when the input has focus
    (`ApiKeyDialog.svelte:25`/`:58`) — move it so Escape closes the dialog
    regardless of focus, add a focus trap while open, and restore focus to
    the trigger element on close. The mobile sidebar overlay
    (`+layout.svelte` `closeDrawer`) has no keyboard dismissal at all — add
    Escape handling there too.

12. **`run.status` display helper.**
    The same inline `replace('_', ' ')` is duplicated at
    `ProbeTable.svelte:104`, `+page.svelte:53`, and
    `probes/[id]/+page.svelte:192`. Replace with one typed helper keyed off
    the status constants. (The separate `provider.kind.replaceAll('_',' ')`
    fallbacks are unrelated — leave those.)

13. **Track `proc-macro-error2` (unmaintained, transitive via `tabled`/`age`).**
    Not a CVE, currently allowed by `cargo audit`. Revisit when upstream
    crates upgrade past it.

14. **No-magic-values sweep (frontend).**
    Audit remaining inline literals in `web/src` (status strings,
    storage-event names, toast durations, JS breakpoints) and route them
    through `constants.ts`.

15. **Magic strings in provider wire contracts (low priority).**
    `role: "user"` (`openai.rs:234`, `azure.rs:286`, `anthropic.rs:162`),
    Anthropic's `block_type == "text"` / `stop_reason == "refusal"` /
    `"end_turn"` (`anthropic.rs:200-210`), URL schemes `"http"`/`"https"`
    (`alert.rs:322`). Arguably acceptable as wire-contract literals, but
    technically magic. The webhook payload keys (`"event_id"`, `"rule_id"`,
    `"drift_level"`, `"fired_at"` — `alert.rs:255-259`) are the more
    valuable fix since they're an outbound contract: a typed payload struct
    would stop them from silently drifting apart from the actual `AlertEvent`
    shape.

16. **`model_version` "first wins" hides within-run disagreement.**
    `probe_runner.rs` records only the first reported model version across
    a run's samples/prompts. If a provider serves a partial rollout
    (different versions mid-run), the disagreement is silently dropped —
    notable since this is specifically the model-version detector. Add a
    `tracing::debug!`/`warn!` when a later sample reports a version that
    differs from the first.

17. **Test-fixture duplication.**
    `timeout_secs: 30` plus a full `AppConfig`/`AppState` literal is
    copy-pasted across 5 route test modules (runs/probes/alerts/baselines/
    vault). A shared `test_state()` helper would remove the duplication and
    the magic `30`.

18. **SSRF allowlist edge ranges — verify, not clearly wrong.**
    `is_disallowed_ip` covers loopback/RFC-1918/link-local/metadata but not
    `100.64.0.0/10` (CGNAT) or `198.18.0.0/15` (benchmark). Realistic
    attacker-controlled targets are already covered; these are completeness
    gaps worth a deliberate decision, not an obvious bug.

19. **Cache-busting field efficacy — confirm against the current API.**
    The varying `user` field defeats exact-request/proxy caching and
    `store: false` prevents storage, but `user` does not key OpenAI's
    *prompt* cache (that's `prompt_cache_key` / the prompt prefix). For
    short probe prompts, prompt-caching likely doesn't apply and sampled
    outputs still vary, so the current approach is defensible — but if
    defeating prompt-level caching specifically becomes a goal,
    `prompt_cache_key` is the precise knob. Confirm against current OpenAI
    docs before changing anything.

20. **Run / event retention.**
    `RunStore`/`AlertRuleStore` grow unbounded — every scheduled run and
    event is stored forever. Add a retention/pruning policy before any
    long-running deployment.

21. **`AlertRuleStore::list_events` still full-scans.**
    Iterates and deserializes every event to return the most recent N. Only
    hit by the manual events route (not the scheduler hot path), so low
    priority — a time-ordered index like `run_index` would make it
    `O(limit)` and would pair naturally with retention (item 20).

22. **Blocking redb I/O on async worker threads — revisit only if profiling shows a hot spot.**
    Store calls are synchronous inside async tasks. After the run-storage
    split and the cooldown index, scheduler-path store ops are µs-scale
    point lookups / bounded range scans, where `spawn_blocking`'s
    thread-handoff would cost more than it saves. Not a blanket wrap;
    revisit only for a genuinely CPU-bound path (e.g. the drift permutation
    test, via `block_in_place`) if profiling shows it matters.

23. **`let out = (…); out` attribute-scoping workaround (trivial).**
    Used only to attach `#[allow(clippy::cast_*)]` to an expression (e.g.
    `assessment.rs:301` `euclidean`, `twosample.rs` `next_bounded`). A
    module-level `#![allow]` (as `assessment.rs` already does for
    `cast_precision_loss`) would remove the boilerplate.

24. **Test gaps.**
    No frontend tests at all. Calibration is empirically validated
    (null-FPR Monte-Carlo) and the scheduler has restart-catch-up and
    shutdown tests, but there are no store concurrency-stress tests.

### Post-v1 — premium tier (do not start before the blocking items above are done)

Additive, not a rewrite — the calibrated engine, storage, scheduler, and
alerting all stay; these feed new signals into the same `DriftReport`/alert
pipeline. Recommended build order is listed (cheapest/most-broadening first,
biggest differentiator last):

25. **Multi-signal detectors (build first).**
    Embedding shift is one axis; "drift monitoring" only becomes fully true
    with more. Add small, pluggable detectors emitting into the existing
    severity/alert path (~a day each): output format/schema validity,
    refusal rate, latency, output-length distribution, sentiment/tone. No
    engine change — each is a new signal source feeding the same report.

26. **Structured expectation/assertion checks.**
    Extend the probe's existing `expected_contains` to structured
    per-prompt assertions (JSON shape, regex, value bounds). Small change;
    turns probes into contract tests as well as drift canaries.

27. **Production-traffic sampling (biggest build, the real differentiator).**
    Today drift is only seen on a handful of synthetic probes. Mirror/sample
    real requests and run the same drift test on them. This is the one
    genuinely new subsystem — an ingestion path plus PII/privacy handling —
    and it's what converts "synthetic canary" into actual production
    monitoring. Still plugs into the existing pipeline; highest effort,
    highest payoff.

---

## Notes

- Greenfield: no production data, so a v1→v2 baseline migration UX is not
  needed — old baselines are simply re-captured.
