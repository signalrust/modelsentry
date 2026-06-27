# ModelSentry — Remaining Work

Status snapshot after the provider-unification + drift-rebuild push (`b5462d2`).
Items are grouped by priority. "Blocking" = prevents a competent user from
completing the main loop (configure → probe → baseline → drift → alert) via the
dashboard.

---

## Recently completed (context — not TODO)

- Unified provider subsystem: self-describing `ProviderSpec`, single per-run
  resolver (`provider_factory::build_provider`), no registry, vault store-only.
- **Azure OpenAI** provider (adapter + `[providers.azure]` config + UI).
- All compile-time constants centralized in `constants` modules (Rust + TS).
- Frontend mirrors `ProviderSpec`; dashboard builds with `adapter-node`; daemon
  is API-only.
- Drift detection rebuilt on a calibrated two-sample foundation (schema v2).
- Docs refreshed (README, ARCHITECTURE, CHANGELOG, config). `@types/node` added.

---

## P1 — Core dashboard workflow (UI-only users are blocked without these)

- [ ] **Baseline capture button (UI).** API + CLI exist; the dashboard only
      *displays* baselines. Add a capture action on the probe detail page with
      loading / error / "needs N runs" states; refresh baseline + drift after.
      (`web/src/routes/probes/[id]/+page.svelte`, `api.baselines.captureForProbe`)
- [ ] **Alert-rule creation (UI).** `api.alerts.createRuleForProbe` exists but is
      never called. Add a form (target FPR + channel: webhook / Slack / email),
      list existing rules, and a delete action.
- [ ] **Anthropic "completions-only, no drift" disclaimer in the UI.** README
      documents it; the new-probe form should also warn when Anthropic is
      selected (parity with the Azure embedding hint). (`AddProbeForm.svelte`)
- [ ] **First-run onboarding panel.** Lightweight checklist when there are no
      probes / no API key (vault passphrase, provider key, dashboard API key).

## P1 — Feature: email notifications

- [ ] **Email alert channel (SMTP).** `AlertChannel::Email` currently logs only.
      Implement real delivery (e.g. `lettre`), add `[alerts.smtp]` config
      (host, port, from, TLS; password via vault), surface failures through the
      existing `Result` path. Unit-test message construction; gate the network
      test. (`crates/core/src/alert.rs`)

## P2 — Statistical rigor (path to 10 on calibration)

- [x] **Sample each prompt N times per run.** DONE — `[alerts] samples_per_prompt`
      (default 3); each prompt scored by a two-sample energy permutation, removing
      the `1/(k+1)` single-prompt floor. (`probe_runner`, nested `ProbeRun.embeddings`,
      baseline capture, `crates/core/src/drift/{assessment,twosample}.rs`.)
- [ ] **Sequential / over-time multiple-testing control.** Every scheduled run is
      a fresh test; `target_fpr` is per-run, so hourly probing accumulates false
      alarms over a month. Add alert de-duplication / cooldown or an alpha-spending
      style control, and state the per-run vs per-period distinction in the UI.
- [ ] **Report an effect size, not just significance.** `−log₁₀(p)` conflates
      magnitude with precision. Add an interpretable drift magnitude (e.g. mean
      standardized excursion / distance in embedding space) alongside the p-value.
- [ ] **Baseline-health check.** Warn when a prompt's baseline cloud has
      near-zero variance (deterministic/cached outputs) — results there measure
      embedding noise, not drift.

## P2 — UX quality

- [ ] **CSS: 100% from theme tokens.** Replace the remaining hardcoded
      colors/rgba/hex in `web/src/app.css` and component `<style>` blocks with
      per-theme tokens: `--on-accent` (the `#fff` literals), `--scrim`
      (sidebar overlay + `ApiKeyDialog` `rgba(0,0,0,0.55)`), soft fills
      (`--fill-up/warn/down/info` for badges/meter/live-badge/error-banner),
      `--focus-ring` (the hardcoded blue focus shadow), `--card-sheen`,
      `--btn-primary-shadow`, and the pulse-keyframe colors. Verify
      `DriftChart` canvas reads chart colors via `getComputedStyle` (re-themes).
- [ ] **Live data refresh / honest LIVE badge.** Pages load once on mount; the
      header shows "LIVE". Either add polling (interval + `onDestroy` cleanup +
      pause on hidden tab / in-flight run) or make the badge reflect real state.
      (`web/src/routes/+layout.svelte`, `+page.svelte`, `probes/+page.svelte`,
      `probes/[id]/+page.svelte`)
- [ ] **Accessibility.** Mobile sidebar overlay should be keyboard-dismissible;
      `ApiKeyDialog` Escape should close regardless of focus, with a focus trap
      and focus restore on close.
- [ ] **`run.status` display helper.** Replace the 3× `replace('_', ' ')` with a
      typed helper keyed off the status constants.

## P3 — Hygiene / docs / deploy

- [ ] **README "proxies /api" fix.** The dev server does not proxy; the browser
      calls `:7740` directly (CORS). Correct the wording.
- [ ] **Release checklist + test counts.** Reconcile `RELEASE_READINESS_CHECKLIST.md`
      and any stale test counts in README before tagging.
- [ ] **Track `proc-macro-error2` (unmaintained).** Transitive via `tabled`/`age`;
      not a CVE, allowed by `cargo audit`. Revisit on upstream upgrades.
- [ ] **No-magic-values sweep (frontend).** Audit remaining inline literals in
      `web/src` (status strings, storage-event names, toast durations,
      breakpoints used in JS) and route them through `constants.ts`. Same sweep
      for any drift weights/thresholds still inline in `crates/core/src/drift`.

---

## Notes

- Greenfield: no production data, so v1→v2 baseline migration UX is **not**
  needed (old baselines are simply re-captured).
- Provider keys are CLI/API only today (no provider-key UI); the onboarding
  panel above is the lightweight path to surface that.

  //--

  27 June audit

  One-line verdict
Unusually rigorous engineering and statistics for a project this size — wrapped around a product whose flagship detector cannot fire at its own default settings, monitoring a signal narrower than it claims. Strong alpha; not a credible launch yet.

Scorecard
Dimension	Score	One-liner
Statistical method (design)	8/10	Correct conformal + MMD/energy + permutation; right tools for n≪d.
Statistical method (calibration in practice)	9/10	FIXED — stratified-permutation gate + per-prompt multi-sampling (two-sample energy); single-prompt floor removed; empirically calibrated (tested). Residual for 10: sequential/over-time control + effect size.
Architecture / design	8/10	Post-refactor: unified ProviderSpec, single resolver, clean layering.
Code quality	9/10	Clippy -D warnings, no-unwrap lint, centralized constants, real docs. Top-decile.
Test quality	7/10	Now validates calibration empirically (null-FPR Monte-Carlo) + broad-drift power; ~no frontend tests still.
Frontend	5/10	Nice design system; incomplete as a control plane; dishonest "LIVE"; theme leaks.
Security hygiene	8/10	age vault, constant-time key compare, SSRF guard, body/rate limits.
Product completeness	4/10	Core loop needs the CLI; email is a stub; onboarding friction.
Market / positioning	4/10	Real pain, crowded field, thin wedge; "self-hosted/private" is the only sharp edge.
Honesty (claims vs. reality)	4/10	Several overclaims (below).
The flagship problem — the default detector was mute → RESOLVED
Original finding: the per-prompt conformal rank is hard-floored at 1/(k+1); the
Šidák-of-min gate inherited that floor, so at the defaults (k=20, target_fpr=0.01)
the preferred method could never fire — only ever "None." Perverse corollary:
a 1-run (pooled) baseline could alert while a 20-run (conformal) one could not.

Fix shipped (crates/core/src/drift/assessment.rs):
- The gate is now a STRATIFIED PERMUTATION test of an aggregate statistic
  T = Σ max(zᵢ, 0) over standardized per-prompt excursions (test position
  resampled within each prompt's augmented set → exact under exchangeability).
  Resolution is 1/(B+1), so broad/model-wide drift (the headline case) clears
  small target FPRs the single-prompt rank could not.
- Per-prompt conformal p-values are retained for ATTRIBUTION (which prompt moved).
- Resolution guard: permutations auto-raised so 1/(B+1) ≤ target_fpr — the gate
  can never be silently un-fireable.
- Variance floor handles near-deterministic (temperature-0/cached) baselines.
- New tests: empirical null-FPR calibration (regression guard for this bug) and
  broad-drift-below-the-floor power.

Honest residual (why 8/10, not 10): with ONE run sample per prompt,
single-prompt-only drift is information-bounded at 1/(k+1) — sharpen by a larger
baseline cloud, or (the real upgrade) sample each prompt multiple times per run.
And there is still no sequential / over-time multiple-testing control (below).

Other statistical issues a referee would raise
"The combined p-value is the false-positive rate" (repeated in docs/config) is overstated: (a) it's per-run, but every scheduled run is a fresh test with no alpha-spending — hourly probing at α=0.01 is ~7 false alarms/probe/month, not "one per 100 runs" in any operator-meaningful sense; (b) Šidák assumes prompt-independence (real drift hits prompts together → positively dependent → conservative, so the exactness claim is wrong even if it errs safe); (c) conformal validity needs the baseline runs to be exchangeable with the test run — provider version pinning, time-of-day, caching, and autocorrelation all break this.
Significance ≠ effect size. statistic = −log₁₀(p) is reported as a "drift score," but it conflates magnitude with precision: a large baseline makes a trivial shift "Critical." There is no interpretable effect size or direction.
Degenerate baselines. Temperature-0 / cached / deterministic prompts → near-zero-variance clouds → the test measures embedding noise; no "baseline variance health" check warns that results are meaningless.
f32 for high-dimensional kernel sums — use f64 for the statistic.
"Pretendings not covered" (honesty gaps)
Default detector can't alert (above) — the big one.
"Detects model behaviour shifts" — it detects semantic-embedding shift only. Format/JSON-validity breakage, latency, tone, refusals (partial), and safety regressions that preserve meaning are invisible.
Synthetic canary probes ≠ production monitoring. You see drift only if it hits your handful of prompts; competitors watch real traffic. Not stated dishonestly, but buyers will assume more.
"LIVE" badge with no polling; email channel is a tracing::info! stub; baseline + alert-rule creation are CLI-only, so the "dashboard" isn't a control plane yet.
Architecture / code / tests
Architecture: genuinely good now — ProviderSpec is self-describing, one resolver, per-run construction, vault store-only. Remaining smells: per-run reqwest::Client rebuild (minor waste), scheduler doesn't persist next-run times (restart re-runs everything; no catch-up), and there's no global concurrency/rate cap across probes (N probes × 4 can stampede a provider).
Code quality: among the best I've reviewed at this scale — lint gates, no-unwrap, error taxonomy, doc density, constants discipline. Little to criticize.
Tests: the gap is qualitative-not-quantitative — they assert "drift→low p, stable→high p" but never simulate the null many times to check empirical FPR ≈ α, which is exactly the test that would have caught the un-fireable default. Plus effectively no frontend tests and no scheduler/store concurrency tests.
Market / utility
Real problem, but a thin wedge in a crowded space (LangSmith, Arize Phoenix, Langfuse, Helicone, Evidently/LangKit, Fiddler, WhyLabs). They bundle tracing, evals, cost, and prompt management; this does one narrow thing. Your one defensible edge is self-hosted, nothing leaves the box for regulated/private deployments — lead with that, not with "more statistics," because the stats advantage is undercut by the calibration bug and the narrow signal.

Launch-readiness résumé
Where it is: Engineering is ~beta. Statistics design is PhD-credible; statistics delivery is broken at defaults. Product is alpha (incomplete dashboard, stub email, mute defaults, untested calibration).

Blocking before any launch:

Fix the default sensitivity / baseline-size relationship and warn when the detector can't resolve target_fpr.
Add an empirical calibration test (simulate the null, assert FPR ≈ α) — and run-cadence guidance / alert de-dup.
Honest scoping in copy: "semantic-embedding drift on synthetic probes, self-hosted."
Complete the dashboard loop (baseline capture + alert rules), real email, polling.
Distance: ~6–10 focused weeks to a credible v1 for the privacy-sensitive self-hosted niche. The hard part isn't the remaining code — it's that the scientific centerpiece needs a calibration correction plus validation before the "calibrated, trustworthy" story is true. Until #1–#2 land, a confident green "No drift" dashboard is the single biggest liability, because it's reassuring and, at defaults, meaningless.

If you want, I can convert the four blockers into concrete tickets in TODO.md — and I'd argue the calibration default belongs at the very top, above the UI work.

//-- AUDIT 2

Done. The fix is implemented, tested, documented, and verified. Here's what changed and the honest new score.

What was wrong
The per-prompt conformal rank is hard-floored at 1/(k+1), and the old Šidák-of-min gate inherited that floor — so at the defaults (k=20, target_fpr=0.01) the preferred detector mathematically could never fire. Worse, a 1-run (pooled) baseline could alert while a 20-run (conformal) one couldn't.

The fix (assessment.rs)
New gate: a stratified permutation test of an aggregate statistic T = Σ max(zᵢ, 0) over standardized per-prompt excursions. Resampling the test position within each prompt's augmented set is exact under the same exchangeability the conformal construction already assumes — so it's valid, needs no independence assumption, uses magnitude (not just rank), and resolves to 1/(B+1). Broad/model-wide drift (the headline case) now clears 0.01 easily.
Per-prompt conformal p-values retained purely for attribution (which prompt moved).
Resolution guard: permutations auto-raise so 1/(B+1) ≤ target_fpr — the gate can never be silently un-fireable.
Variance floor so near-deterministic (temperature-0/cached) baselines don't divide by ~0.
New tests: an empirical null-FPR calibration test (the regression guard that would have caught the original bug) and a broad-drift-below-the-floor power test.
Docs updated: methodology §8 (the one place that was now wrong), CHANGELOG, config comment; TODO scorecard updated with honest residuals.

Verification
cargo clippy --workspace --all-targets -- -D warnings — clean
cargo test --workspace — 200 pass, 0 fail (incl. the new calibration/power tests)
Honest new score: calibration in practice → 8/10 (was 3/10)
I won't inflate it to 10, because as your critic I'd flag the same things:

With one run sample per prompt, single-prompt-only drift is information-bounded at 1/(k+1) — that's a property of one observation, not the test. The real 9–10 fix is sampling each prompt N times per run (now a P2 backlog item).
No sequential / over-time multiple-testing control yet (per-run FPR ≠ per-month), and −log₁₀(p) still conflates significance with effect size. Both are now in TODO.md under "path to 9–10."
So: the flagship "mute detector" problem is genuinely resolved and proven calibrated; the remaining two points require a sampling change and sequential control, which are real scope, not a one-line tweak. I left everything uncommitted. Want me to take the N-samples-per-prompt change next (that's what closes the gap to 9–10), or commit this fix first?
