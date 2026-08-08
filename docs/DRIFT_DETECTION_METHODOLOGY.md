# Statistical Drift Detection — Methodology

Kernel two-sample testing and conformal prediction applied to LLM output
embeddings, with a stratified permutation gate for cross-prompt aggregation
and rolling-window alpha-spending for sequential control. Assumes familiarity
with hypothesis testing, kernel methods, and nonparametric statistics.

> **Scope.** This is drift detection on the synthetic probes you configure —
> not production-traffic monitoring, and not a check on format/JSON validity,
> latency, tone, or refusals (see the project
> [README § Limitations](../README.md#limitations)). Validity assumes
> baseline/run *exchangeability*; status of the known breakers is in §10.

Implementation: `crates/core/src/drift/{twosample,assessment}.rs`,
`crates/core/src/alert.rs` (§8).

---

## Contents

1. [Setup](#1-setup) · 2. [Pooled two-sample statistic](#2-pooled) ·
3. [Per-prompt conformal attribution](#3-conformal) ·
4. [Aggregation: stratified permutation gate](#4-gate) ·
5. [Effect size](#5-effect-size) · 6. [Severity mapping](#6-severity) ·
7. [Power and baseline sizing](#7-power) ·
8. [Sequential control](#8-sequential) · 9. [Pipeline](#9-pipeline) ·
10. [Exchangeability status](#10-exchangeability) · [References](#references)

---

<a name="1-setup"></a>
## 1. Setup

Baseline `B = {b₁,…,b_m}` and run `R = {r₁,…,r_n}`: output-embedding samples
from a fixed prompt corpus, one embedding per **completion**. Embedding the
prompt text itself is invariant under a fixed embedding model and carries no
signal — only completions are embedded. Question: are `B` and `R`
exchangeable under a common distribution, or has the model's conditional
output distribution shifted?

A completion is excluded from the sample rather than embedded if the
provider reports it as truncated (`finish_reason: length`) or content-filtered
— either follows a different generation process than a complete response and
would corrupt the two-sample comparison (`reject_unusable_finish_reason`,
`crates/core/src/provider/mod.rs`).

<a name="2-pooled"></a>
## 2. Pooled two-sample statistic (fallback path)

Used when any prompt's baseline cloud has `< 2` samples (`MIN_SAMPLES_PER_GROUP
= 2`); otherwise superseded by §3–4.

- **MMD²**, unbiased U-statistic (Gretton et al., *JMLR* 2012):
  `MMD²(B,R) = (1/m(m−1))Σ_{i≠j}k(bᵢ,bⱼ) + (1/n(n−1))Σ_{i≠j}k(rᵢ,rⱼ) −
  (2/mn)Σ_{i,j}k(bᵢ,rⱼ)`. Kernel: RBF, `k(x,y) = exp(−‖x−y‖²/2σ²)`, `σ` set
  by the median heuristic over pooled pairwise distances (floored at
  `BANDWIDTH_FLOOR = 1e-6`).
- **Energy distance** (Székely & Rizzo, *JSPI* 2013): identical construction
  with `k(x,y) = −‖x−y‖`, biased form (diagonal is exactly 0, so the
  unbiased correction is moot). Parameter-free — the kernel used for the
  per-prompt path (§3–4).

Rejected alternative: a parametric Gaussian-KL divergence needs each cloud's
`d×d` covariance; `d ∈ {1536, 3072}` while a run yields `O(1–10)` samples per
prompt (`n ≪ d`), so the estimate is singular. MMD/energy are nonparametric
and valid at `n ≪ d`.

**Calibration.** Permutation test over the pooled `m+n` points
(`permutation_nulls`): `n_permutations` relabelings (default 200,
auto-raised — see §4), statistic recomputed each time, seeded with a
dependency-free SplitMix64 PRNG for reproducibility. `p = (1 + #{T* ≥
T_obs − ε}) / (1 + n_permutations)`, `ε = PERMUTATION_TOLERANCE = 1e-6`
(floating-point tie tolerance). Under `H0`, `p` is uniform on the
permutation grid, so `p < α ⇒ FPR ≈ α` by construction.

<a name="3-conformal"></a>
## 3. Per-prompt conformal attribution

Preferred path (`≥2` baseline samples/prompt): one-vs-cloud test per prompt,
reported for **attribution** (which prompt moved), not the alert gate itself.

- Nonconformity score: mean Euclidean distance to the other `k` points,
  computed over the augmented set `{test point} ∪ cloud`, each of the `k+1`
  points scored against the *other* `k` — the equal-reference-set-size
  condition for exact conformal validity under exchangeability (Vovk,
  Gammerman & Shafer, 2005; Lei et al., *JASA* 2018).
- `p = (1 + #{cloud scores ≥ test score}) / (k+1)` — exact, distribution-free,
  valid for any `k`, any distribution, under baseline/test exchangeability.
  Floor: `1/(k+1)`.

<a name="4-gate"></a>
## 4. Aggregation: stratified permutation gate

The `1/(k+1)` rank floor is too coarse to serve as the alert gate at
realistic `k` (`k=20 ⇒ 1/21 ≈ 0.048`, unfireable at `target_fpr = 0.01`).
The gate instead aggregates *magnitude* across prompts:

1. Standardize each prompt's score distribution to zero-mean/unit-variance
   (SD floored at `STD_FLOOR = 1e-6` for near-deterministic baselines) →
   per-prompt null pool + observed excursion `zᵢ`.
2. `T = Σᵢ max(zᵢ, 0)`.
3. Null for `T`: resample one draw per prompt from its own pool
   (`stratified_permutation_p`) — valid because each prompt's draws are
   exchangeable *within* its own stratum; strata need only be independent
   across prompts, not identically distributed, which within-prompt
   resampling preserves even under the heterogeneity below.
4. `combined_p = (1 + #{T* ≥ T_obs − ε}) / (1 + B)`.

**Strata are path-dependent.** Per prompt, the score source depends on
sample count (`prompt_score`):
- `≥2` run samples (`[alerts] samples_per_prompt`, default 3): two-sample
  energy permutation, baseline cloud vs. run-sample cluster. Null pool
  *excludes* the observed point.
- `1` run sample: rank-based conformal augmented scores (§3). Null pool
  *includes* the observed point at index 0.

The `1/(B+1)` floor is a property of the `(1+at_least)/(1+B)` estimator, not
of pool composition — it holds identically on both paths.

**Multi-sampling removes the single-prompt floor.** With `samples_per_prompt
≥ 2`, a cleanly separated run cluster makes the observed two-sample energy
the strict maximum over every within-prompt relabeling, so a *single*
drifted prompt alone can drive `combined_p` to `1/(B+1)` — the `1/(k+1)`
rank floor no longer bounds single-prompt drift. At `samples_per_prompt=1`
the rank-conformal floor still governs the single-prompt-only case.

**Resolution guarantee.** `B` is auto-raised (`min_perms_for_resolution`,
`⌈1/target_fpr⌉`) so `1/(B+1) ≤ target_fpr` — the gate is never silently
unable to reach the configured level, independent of baseline size. Applied
identically to the pooled fallback (§2).

Rejected alternatives: Šidák-of-min (Šidák, *JASA* 1967) inherits the
`1/(k+1)` floor and assumes prompt independence; Fisher's method dilutes one
strong signal among quiet ones. Neither is used for the gate — cited here
only as the alternatives considered and rejected.

If any prompt's baseline cloud has `< 2` samples, the run falls back
entirely to §2 rather than mixing gate types.

<a name="5-effect-size"></a>
## 5. Effect size

A p-value conflates *evidence accumulated* with *magnitude of shift* — a
large baseline drives `p → 0` for an arbitrarily small, practically
immaterial change. `DriftAssessment.effect_size` reports magnitude
independent of that: the standardized positive excursion `(observed − null
mean)/null SD` (floored, non-negative), averaged across prompts in
per-prompt mode (`Σzᵢ⁺ / |prompts|`) or the pooled statistic's own excursion
in fallback mode. Units: null standard deviations: `~0` ⇒ within noise.

Not yet implemented: a *direction* in embedding space (magnitude only today
— tracked as an optional extension, `docs/TODO.md`).

<a name="6-severity"></a>
## 6. Severity mapping

`drift_score = −log₁₀(combined_p)` (floored to avoid `∞` at `p=0`).
Severity relative to `α = target_fpr`:

| `combined_p` | Level |
|---|---|
| `p ≥ α` | None |
| `α/10 ≤ p < α` | Low |
| `α/100 ≤ p < α/10` | Medium |
| `α/1000 ≤ p < α/100` | High |
| `p < α/1000` | Critical |

<a name="7-power"></a>
## 7. Power and baseline sizing

The `1/(k+1)` conformal floor (§3) bounds achievable significance by cloud
size `k` alone — `k` in the low hundreds is needed to resolve `α = 0.01`
for single-prompt-only drift under `samples_per_prompt = 1`. Baseline
capture aggregates `baseline_capture_runs` prior runs into a growing
per-prompt cloud (default 20); power scales with `k`. Below `k=2` the
pipeline falls back to §2 and reports that in `DriftAssessment.method`.

<a name="8-sequential"></a>
## 8. Sequential control (rolling-window alpha-spending)

`target_fpr` is a per-run level; repeated looks accumulate `E[false alarms]
= Σ_looks P(fire | H0) = Σ_looks(level of that look)` — the standard
multiple-looks problem. `[alerts.sequential]` (`window_secs`,
`alpha_budget`) bounds this sum per rule per rolling window.

Structurally analogous to Lan–DeMets alpha-spending functions (Lan & DeMets,
*Biometrika* 1983), adapted to a rolling window and an **expected-count**
target rather than the fixed-horizon cumulative Type-I error of the
original clinical-trials framing. Debit-on-look: each run spends `α_run =
min(target_fpr, alpha_budget − spent_in_window)` (never negative),
persisted per-rule in a chronologically-keyed, window-pruned ledger
(`AlphaSpendStore`, `crates/store/src/spend_store.rs`), so the guarantee
survives daemon restarts. The per-window spend telescopes to exactly
`alpha_budget`; once exhausted the rule tests at level 0 until older spends
age out of the window.

This bounds `E[# false alarms in the window]` — not `P(≥1 false alarm)`
(FWER) and not the proportion of alarms that are false (FDR). A true drift
episode also spends budget, so a rule can go silent for the remainder of
the window after a real incident — by design; the guarantee is about false
alarms, not about repeated confirmation of a real one.

Distinct from `[alerts] cooldown_secs` (default 3600): cooldown
de-duplicates a *burst* of repeated fires within a short, reset-on-fire
window (noise suppression, no error-rate bound); sequential control bounds
the *count* over the full rolling window regardless of firing pattern. They
compose. `alpha_budget ≤ 0` disables sequential control (the default).

Not yet implemented: dashboard surfacing of per-rule spend vs. remaining
budget — the backend guarantee is complete; only the UI display is pending
(`docs/TODO.md`).

<a name="9-pipeline"></a>
## 9. Pipeline

```
baseline capture: K runs × P prompts → per-prompt cloud (K samples each)
scheduled run:
  1. complete each prompt (samples_per_prompt× when configured) → embed
  2. per prompt: conformal p (§3, attribution) + standardized excursion zᵢ
     [any prompt at k<2 ⇒ whole run falls back to pooled test, §2]
  3. gate: stratified permutation p of T=Σmax(zᵢ,0) → combined_p (§4)
  4. severity from combined_p vs target_fpr (§6); effect_size (§5)
  5. alert iff combined_p < α_run; α_run capped by remaining
     sequential-control budget when [alerts.sequential] is set (§8)
  6. persist DriftReport (statistic, combined_p, effect_size, per-prompt
     breakdown, method, model_version_changed)
```

<a name="10-exchangeability"></a>
## 10. Exchangeability — status

Validity of §2–4 assumes baseline/run exchangeability under `H0`. Known
breakers:

| Breaker | Status |
|---|---|
| Provider model-version change | Detected — `Completion.model_version` recorded per run/baseline, compared, flagged in `DriftReport.model_version_changed` |
| Exact-request/proxy caching | Mitigated — per-call cache-busting nonce on cloud-provider requests (`cache_bust_nonce`) |
| Time-of-day effects | Not engineered, deliberately — stratifying baselines by time window trades away power for a confounder of unproven magnitude in LLM APIs; documented caveat only |
| Autocorrelation across consecutive runs | Not handled |

See [README § Limitations](../README.md#limitations).

---

<a name="references"></a>
## References

- A. Gretton, K. Borgwardt, M. Rasch, B. Schölkopf, A. Smola. "A Kernel
  Two-Sample Test." *JMLR*, 2012.
- G. Székely, M. Rizzo. "Energy statistics: A class of statistics based on
  distances." *J. Stat. Plan. Inference*, 2013.
- V. Vovk, A. Gammerman, G. Shafer. *Algorithmic Learning in a Random
  World.* 2005.
- J. Lei, M. G'Sell, A. Rinaldo, R. Tibshirani, L. Wasserman.
  "Distribution-Free Predictive Inference for Regression." *JASA*, 2018.
- Z. Šidák. "Rectangular confidence regions for the means of multivariate
  normal distributions." *JASA*, 1967. (rejected alternative, §4)
- K.K.G. Lan, D.L. DeMets. "Discrete sequential boundaries for clinical
  trials." *Biometrika*, 1983. (alpha-spending, adapted in §8)
- B. Efron, R. Tibshirani. *An Introduction to the Bootstrap.* 1993.
  (resampling background)

These are theorems and estimators, not benchmarks — none are superseded by
later work; a citation year marks provenance, not expiry. What actually
dates in this project is the empirical layer (provider model IDs, embedding
dimensionality), which is config-driven and kept current independently of
the math. See [`ARCHITECTURE.md`](ARCHITECTURE.md) for system design; for
derivations, the source itself — `twosample.rs`, `assessment.rs`, `alert.rs`.
