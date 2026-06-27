# Tuning Drift Detection — Target FPR & Baseline Richness

ModelSentry no longer asks you to tune opaque KL/cosine thresholds. Drift is now
a **calibrated statistical test**: you choose a single, meaningful number — your
**target false-positive rate (FPR)** — and you make the test *powerful* by
capturing a **rich baseline**. This guide covers both. For the full theory, see
[`DRIFT_DETECTION_METHODOLOGY.md`](DRIFT_DETECTION_METHODOLOGY.md).

---

## 1. The one knob: `target_fpr`

Every run is compared to its baseline with a nonparametric two-sample test that
produces a **calibrated combined p-value**. A run alerts when:

```
combined_p_value < target_fpr
```

Because the p-value is calibrated, `target_fpr` *is* your false-positive rate by
construction: with `target_fpr = 0.01`, a model that has **not** drifted will
trip an alert only about **1 run in 100**. There is no separate "KL" and "cosine"
threshold to balance — one number, with a precise meaning.

```toml
[alerts]
# A run alerts when its calibrated combined p-value falls below this.
# It IS your false-positive rate. Must be in the open interval (0, 1).
target_fpr = 0.01
```

| `target_fpr` | Behaviour | ~False alarms on a stable probe |
|---|---|---|
| `0.05` | Sensitive — catches subtler drift | ~1 in 20 runs |
| **`0.01`** | Balanced (default) | ~1 in 100 runs |
| `0.001` | Conservative — only strong evidence | ~1 in 1000 runs |

Lower `target_fpr` ⇒ fewer false alarms, but you also need more baseline data to
retain the **power** to detect real drift at that stricter bar (see §3).

You can also set `target_fpr` **per probe** on an alert rule, overriding the
global default for noisy or especially critical probes.

---

## 2. Severity bands

The same calibrated p-value drives the reported **drift level**, defined relative
to `target_fpr` (written α below) by orders of magnitude:

| Combined p-value | Drift level |
|---|---|
| `p ≥ α` | **None** (within normal noise) |
| `α/10 ≤ p < α` | **Low** |
| `α/100 ≤ p < α/10` | **Medium** |
| `α/1000 ≤ p < α/100` | **High** |
| `p < α/1000` | **Critical** |

So severity is not an arbitrary multiplier ladder — it is "how many orders of
magnitude past your chosen false-positive rate did this run fall."

---

## 3. Power comes from the baseline, not the threshold

A baseline stores, **per prompt**, a *cloud* of past output (completion)
embeddings. The conformal test compares each new run's answer for a prompt
against that prompt's cloud. The smallest p-value the test can possibly report
for a single drifted prompt is `1 / (cloud_size + 1)`:

- A baseline captured from **1 run** has one sample per prompt → the test falls
  back to a lower-power **pooled** mode and can only catch *gross* drift. The
  report says so (`method = pooled_two_sample`).
- A baseline aggregated over **many runs** has a deep cloud per prompt → real
  power to flag subtle, single-prompt regressions at a strict `target_fpr`.

Capture richness is controlled by:

```toml
[alerts]
# How many recent successful runs to fold into a baseline capture.
baseline_capture_runs = 20
```

**Workflow:** let the probe run on its schedule until you have a good number of
clean runs (no intentional model/prompt changes), *then* capture:

```bash
modelsentry baseline capture <probe-id>
```

The capture aggregates up to `baseline_capture_runs` of the most recent runs into
per-prompt clouds. More clean runs before you capture ⇒ more power. To detect
subtler drift later, capture again once more clean history has accumulated.

> **Honesty note.** No setting conjures power from too little data. With a
> single-run baseline you can only catch large shifts, and the tool tells you
> (large p-values, `pooled_two_sample` method). The statistics are honest about
> uncertainty — that honesty is the point.

---

## 4. When to re-capture the baseline

Re-capture (rather than loosen `target_fpr`) whenever *current-normal* changes:

- After **intentionally upgrading the chat model** under test — the new model's
  normal output legitimately differs from the old baseline.
- After **editing a probe's prompts** — the prompt set defines the baseline.
- After **changing the embedding model** — *mandatory* (see §5).
- When a probe is **chronically noisy** and the noise is genuine model behaviour
  (raise that probe's `target_fpr` too, or capture a richer baseline so the
  natural spread is represented in the cloud).

Loosen `target_fpr` only when alerts are noisy due to a deliberately *high*
sensitivity; tighten it when known regressions never fire (and capture a richer
baseline so the tighter bar still has power).

---

## 5. Re-capturing after an embedding-model change

The test is only valid when the run and the baseline were produced by the **same
embedding model**. `text-embedding-3-small` (1536 dims) and
`text-embedding-3-large` (3072 dims) are not interchangeable.

When you switch `embedding_model` / `embedding_dim` under `[providers.openai]`,
existing baselines become incompatible. ModelSentry detects this and **skips
drift detection** for affected probes, logging an actionable warning
(`re-capture the baseline for this probe`) rather than silently comparing
incompatible vectors. Baselines captured under the old **schema** (pre-cloud) are
likewise rejected with a re-capture message.

**To recover:** re-capture each affected probe's baseline under the new model:

```bash
modelsentry baseline capture <probe-id>
```

---

## 6. Reading the verdict

Each report carries a plain-language **interpretation** of the *statistical*
result — which test ran, the calibrated p-value vs your target FPR, and the
single prompt that moved most. For example:

> **High drift:** the per-prompt conformal test rejects the no-drift hypothesis
> (combined p = 0.0003 < target FPR 0.0100), so the model's outputs have shifted
> relative to the baseline. Strongest signal: prompt #2 (p = 0.0003, baseline
> n = 40).

The dashboard's *Latest Metrics* panel shows the combined p-value, the target
FPR, the drift score (`−log₁₀ p`), the method, and the **per-prompt breakdown**
so you can see *which* prompt regressed. This is a description of what was
actually measured — not a guess at the semantic meaning of the change. (Semantic
explanation would require a separate LLM-as-judge pass, a possible future
opt-in.)

---

## See also

- [`DRIFT_DETECTION_METHODOLOGY.md`](DRIFT_DETECTION_METHODOLOGY.md) — the full
  statistical theory (MMD, energy distance, conformal prediction, Šidák).
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — system design and engineering standards.
- [`../config/default.toml`](../config/default.toml) — annotated `[alerts]` config.
