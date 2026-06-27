# Statistical Drift Detection — A Complete Guide

**Audience:** anyone, including readers with no statistics or machine-learning
background. Every term is defined the first time it appears and again in the
[Glossary](#glossary). Read top to bottom for the full story, or jump to the
[Glossary](#glossary) to look up a term.

> **One-sentence summary.** ModelSentry watches an LLM by sending it a fixed set
> of prompts on a schedule, turning each *answer* into a list of numbers (an
> *embedding*), and using a rigorous statistical test to decide whether today's
> answers come from the *same distribution* as a trusted *baseline* — alerting
> only when the evidence is strong enough to keep false alarms at a rate you
> choose.

---

## Table of contents

1. [The problem: what "drift" is and why it matters](#1-the-problem)
2. [Embeddings: turning text into geometry](#2-embeddings)
3. [The critical fix: measure outputs, not inputs](#3-outputs-not-inputs)
4. [The core question: are two samples from the same distribution?](#4-two-sample)
5. [Measuring how different two distributions are (MMD & energy distance)](#5-measuring)
6. [From a number to a decision: p-values and the permutation test](#6-pvalues)
7. [Comparing prompt-by-prompt: conformal prediction](#7-conformal)
8. [Combining evidence across prompts: the Šidák correction](#8-combining)
9. [Turning a p-value into a severity](#9-severity)
10. [Statistical power and why baselines should span many runs](#10-power)
11. [Putting it together: the full pipeline](#11-pipeline)
12. [Glossary](#glossary)
13. [References and further reading](#references)

---

<a name="1-the-problem"></a>
## 1. The problem: what "drift" is and why it matters

You build a product on top of a Large Language Model (an **LLM** — a program that
generates text, like GPT or Claude). The model lives behind an API you don't
control. Over time, the provider may silently change it: a new version, a safety
update, a quantization, a routing change. Your prompts are unchanged, but the
*answers* shift. That shift is **drift**.

Drift is dangerous precisely because it is *silent*. Nothing errors out. The API
keeps returning 200 OK. But your summaries get terser, your classifier's labels
skew, your assistant's tone changes, a jailbreak that was patched comes back.
You find out from users, not logs.

**Goal of drift detection:** automatically and continuously answer the question
*"is the model still behaving the way it did when I last trusted it?"* — and
raise an alert when the answer is "no," without crying wolf so often that you
start ignoring the alerts.

That last clause is the hard part, and it's where statistics earns its keep.

---

<a name="2-embeddings"></a>
## 2. Embeddings: turning text into geometry

Computers can't compare sentences directly, but they can compare *numbers*. An
**embedding** is a way to turn a piece of text into a fixed-length list of
numbers — a **vector** — such that *texts with similar meaning get similar
vectors*.

Think of it as giving every sentence an address in a very high-dimensional
space. "The cat sat on the mat" and "A feline rested on the rug" land near each
other; "Quarterly revenue rose 4%" lands far away. The list might have **1536**
numbers (for OpenAI's `text-embedding-3-small`) or **3072** (for
`text-embedding-3-large`). Each number is one **dimension** — one coordinate of
the address.

Two facts about embeddings we will lean on:

- **Distance ≈ dissimilarity.** Two texts that mean different things produce
  vectors that are far apart (large **Euclidean distance** — the ordinary
  straight-line distance, generalized to many dimensions).
- **Direction ≈ topic/meaning.** The *angle* between two vectors captures
  semantic similarity. Models like OpenAI's normalize every vector to length 1,
  so only direction carries information — which is a detail that broke the old
  design (next section).

We embed text so that "did the answers change?" becomes a *geometric* question:
"did the cloud of answer-points move?"

---

<a name="3-outputs-not-inputs"></a>
## 3. The critical fix: measure outputs, not inputs

Here is the single most important idea, and the bug that motivated this rewrite.

A probe is a **fixed** list of prompts (questions). To detect *model* drift you
must look at what changes when the model changes — that is, the model's
**answers** (the **completions**). The **prompts never change**, so embedding the
prompts measures nothing: the same question always produces the same embedding
(for a fixed embedding model). The original implementation embedded the prompts.
Result: the "drift" signal was frozen at zero by construction.

**The fix:** for each prompt, take the model's *answer*, embed *that*, and track
how the cloud of answer-embeddings moves over time. Now the geometry actually
reflects model behavior.

```
   prompt (fixed)            model            completion (varies!)        embedding
"Summarize relativity"  ─►  [ LLM ]  ─►  "Einstein's theory says…"  ─►  [0.01, -0.3, …]
                                              ▲ this is what we embed and watch
```

---

<a name="4-two-sample"></a>
## 4. The core question: are two samples from the same distribution?

We now have two collections of answer-embeddings:

- the **baseline** — answers captured when you trusted the model, `B = {b₁, …, bₘ}`
- the **run** — answers from the latest scheduled check, `R = {r₁, …, rₙ}`

Each is a **sample**: a finite set of points drawn from some underlying,
unknown **probability distribution** (think of a distribution as the "true,
infinite cloud" of all answers the model *could* give; a sample is a handful of
points you actually observed).

The statistical question is the **two-sample problem**:

> Given sample `B` and sample `R`, is it plausible they were drawn from the *same*
> distribution, or is `R` drawn from a *different* one (i.e., the model drifted)?

This is a classic, well-studied problem. We never get a yes/no certainty — only
*evidence*, quantified as a probability (a p-value, §6). The two ingredients are:

1. a **statistic** — a single number measuring how different the two samples look
   (§5), and
2. a **calibration** — a way to say how surprising that number is if there were
   *no* real difference (§6).

---

<a name="5-measuring"></a>
## 5. Measuring how different two distributions are

### 5.1 The naive idea and why it's not enough

The obvious move is to average each cloud into its center point (its
**centroid** — the mean vector) and measure the distance between the two
centroids. This catches a shift of the *average* answer, but it is blind to
changes in *spread* or *shape*: a model could become far more erratic while
keeping the same average, and the centroids wouldn't budge. We want a measure
that compares the whole clouds, not just their centers.

### 5.2 Maximum Mean Discrepancy (MMD)

**MMD** is a principled distance between two *distributions* given only samples
from them (Gretton et al., *A Kernel Two-Sample Test*, 2012). The intuition:

- Pick a **kernel** — a function `k(x, y)` that measures similarity between two
  points, high when they're close, low when far. We use the **RBF kernel**
  (Radial Basis Function, a.k.a. Gaussian kernel):
  `k(x, y) = exp(−‖x − y‖² / (2σ²))`. It equals 1 when `x = y` and decays
  smoothly toward 0 as the points separate. The width `σ` (sigma) sets the
  "reach" of the similarity.
- A kernel secretly maps every point into an even richer space and compares
  *averages* there. MMD is the distance between the two clouds' averages **in
  that richer space**. Crucially, with a good ("characteristic") kernel, that
  distance is **zero if and only if the two distributions are identical** — so
  MMD sees differences in mean, spread, *and* shape, not just the center.

The number we compute (the **unbiased estimator** of squared MMD) is:

```
MMD²(B, R) =  avg over i≠j of k(bᵢ, bⱼ)        (how self-similar B is)
           +  avg over i≠j of k(rᵢ, rⱼ)        (how self-similar R is)
           −  2 · avg over i,j of k(bᵢ, rⱼ)     (how cross-similar B and R are)
```

If `B` and `R` come from the same distribution, the "within" similarities and
the "cross" similarities are about equal, so `MMD² ≈ 0`. If they differ, points
are more similar to their own group than to the other, and `MMD² > 0`.

**Choosing σ — the "median heuristic."** The RBF kernel needs a width `σ`. A
robust, parameter-free default is to set `σ` to the **median** (the middle value)
of all pairwise distances in the pooled data. This auto-scales the kernel to the
data, so you don't hand-tune it.

### 5.3 Energy distance (a parameter-free cousin)

**Energy distance** (Székely & Rizzo) measures the same idea with no width to
choose. It turns out to be exactly MMD using the kernel `k(x, y) = −‖x − y‖`
(negative distance), so our code computes both with one engine. Its formula:

```
Energy(B, R) =  2 · avg over i,j of ‖bᵢ − rⱼ‖   (cross distances)
             −      avg over i,j of ‖bᵢ − bⱼ‖   (within-B distances)
             −      avg over i,j of ‖rᵢ − rⱼ‖   (within-R distances)
```

Zero when the distributions match, positive when they differ. We offer both;
energy distance is a good robust default, RBF-MMD a good sensitive one.

### 5.4 Why not "covariance-aware Gaussian KL"?

A tempting alternative is to model each cloud as a multi-dimensional bell curve
(a **Gaussian**) and compute the **KL divergence** between them in closed form.
This needs each cloud's **covariance matrix** — a `d × d` table (with `d = 1536`
or `3072`) describing how the dimensions vary together. To estimate that table
you need *far more* sample points than dimensions. A probe yields only a handful
of answers per run (`n ≪ d`), so the covariance is **singular** (mathematically
degenerate) and the KL is unstable or undefined. **Kernel methods (MMD/energy)
are nonparametric** — they make no Gaussian assumption and work fine when you
have fewer points than dimensions. That's why they're the right tool here, and
why the old norm-based KL (which dodged the problem by collapsing each vector to
its length, throwing away almost all the information) was not defensible.

---

<a name="6-pvalues"></a>
## 6. From a number to a decision: p-values and the permutation test

`MMD² = 0.012` — is that a lot? We can't know in the abstract; it depends on the
data's natural variability. We need to convert the raw statistic into a
**calibrated probability**.

### 6.1 What a p-value is

A **p-value** answers: *"If there were no real drift, how often would I see a
statistic at least this extreme, just by chance?"* A small p-value (say 0.002)
means "this would almost never happen by luck, so something real is going on." A
large one (0.4) means "this is well within normal noise."

The beauty of p-values: if you decide to alert whenever `p < α` (alpha, your
chosen threshold, e.g. 0.01), then — *when there is no real drift* — you will
raise a false alarm only about `α` of the time. So **α is your false-positive
rate (FPR), chosen on purpose.** This is what "calibrated to a target FPR" means,
and it's the property the old arbitrary "1×/2×/4×/8× threshold" severity ladder
never had.

### 6.2 The permutation test (how we get the p-value)

We don't assume any formula for the "no-drift" distribution of the statistic — we
*build it from the data* with a **permutation test**:

```
1. Compute the observed statistic S on the real split (baseline vs run).
2. Pool all m + n points into one bag, forgetting their labels.
3. Many times (say 200):
      a. Randomly deal the pooled points back into two groups of size m and n.
      b. Compute the statistic on this fake split.
   These fake statistics form the "null distribution" — what the statistic looks
   like when there is, by construction, no real difference.
4. p-value = (1 + how many fake statistics ≥ S) / (1 + number of permutations)
```

If the real split is unremarkable, `S` sits in the middle of the fake ones and
`p` is large. If the real split is special (true drift), `S` exceeds almost all
fakes and `p` is tiny. Under the no-drift hypothesis this p-value is
mathematically guaranteed to be roughly uniform on `[0, 1]`, which is exactly why
thresholding at `α` gives FPR ≈ `α`.

We seed the randomness so the same data always yields the same p-value
(reproducible alerts and tests).

---

<a name="7-conformal"></a>
## 7. Comparing prompt-by-prompt: conformal prediction

### 7.1 Why per-prompt

A probe's prompts are deliberately different ("Summarize relativity," "Capital of
France?"). Their answer-embeddings live in *different regions* of the space.
Pooling them into one big cloud mixes apples and oranges: the large differences
*between* prompts can swamp the small but real drift *within* a prompt. The fix is
to compare **each prompt against its own history**.

### 7.2 One new point vs. a cloud: conformal p-values

For a given prompt, the baseline is a **cloud** of past answer-embeddings (from
many baseline runs — see §10), and the run gives **one** new answer-embedding. We
ask: *is this new point a normal member of the cloud, or an outlier?* This is a
one-vs-many test, and **conformal prediction** gives an exact, assumption-light
p-value for it (Vovk et al.; Lei et al., *JASA* 2018).

The recipe:

1. Define a **nonconformity score** — a number that's large when a point is
   "weird" relative to the cloud. We use the point's **mean distance to all cloud
   points** (far from everyone ⇒ weird).
2. Compute that score for the **new run point**.
3. Compute it for **each baseline point**, measured against the *other* baseline
   points (this is the "leave-one-out" calibration — each baseline point pretends
   to be the new one).
4. The conformal p-value is simply the **rank**:
   `p = (1 + how many baseline scores are ≥ the run's score) / (cloud size + 1)`.

If the new point is typical, its score is middling, many baseline points score
higher, and `p` is large. If it's an outlier, its score tops everyone and `p` is
tiny. This rank-based p-value is **valid for any cloud size and any
distribution** — no Gaussian assumptions — which is why conformal prediction is
the gold standard for "is this new observation anomalous?"

The smallest p-value you can get is `1 / (cloud size + 1)`. With a 9-point cloud
that's 0.1 — not significant. This is the concrete reason baselines need many
samples (§10): **resolution**. More baseline points ⇒ smaller achievable p ⇒
ability to flag subtle drift.

---

<a name="8-combining"></a>
## 8. Combining evidence across prompts: the Šidák correction

Now each prompt has its own p-value `p₁, …, p_P`. We need **one** number for the
whole run. This is a "multiple comparisons" problem: if you run many tests and
alert on the smallest p, you'll get false alarms more often than `α` (more
lottery tickets ⇒ more chance one wins by luck). We must correct for that.

We use the **Šidák correction** on the minimum p-value:

```
combined_p = 1 − (1 − p_min) ^ P
```

where `p_min` is the smallest per-prompt p-value and `P` is the number of
prompts. This inflates the smallest p to account for having looked at `P` of
them, restoring the promised false-positive rate. It is **powerful when a single
prompt drifts hard** — exactly the common real failure (one capability breaks)
— which a naive average would wash out.

> **Why not Fisher's method?** Fisher's method (`−2 Σ ln pᵢ`) is the textbook way
> to *combine* p-values, and it's great when *many* prompts drift a little. But it
> *dilutes* the case where *one* prompt drifts a lot among many quiet ones —
> precisely the alert we care most about. Our own unit test caught this, so we use
> Šidák (sensitive to any single prompt) for the alert decision.

If the data is too thin for per-prompt testing (single-run baseline, §10), we
fall back to the pooled MMD/energy permutation test (§6), which yields its own
calibrated p-value. Either way the run ends with one number: `combined_p`.

---

<a name="9-severity"></a>
## 9. Turning a p-value into a severity

`combined_p < target_fpr` decides *whether* to alert. We also report *how bad* it
looks, by how many orders of magnitude below the threshold the p-value falls:

| Combined p-value | Severity |
|---|---|
| `p ≥ α` | **None** (within normal noise) |
| `α/10 ≤ p < α` | **Low** |
| `α/100 ≤ p < α/10` | **Medium** |
| `α/1000 ≤ p < α/100` | **High** |
| `p < α/1000` | **Critical** |

This replaces the old arbitrary multiplier ladder with a scale anchored to the
calibrated false-positive rate `α` you configured.

---

<a name="10-power"></a>
## 10. Statistical power and why baselines should span many runs

**Statistical power** is the probability of catching real drift when it exists
(the opposite failure from a false alarm). Power grows with sample size. Two
levers:

- **Baseline cloud size per prompt.** As shown in §7, the smallest p you can
  report is `1/(K+1)` for a cloud of `K` baseline answers. To alert at `α = 0.01`
  you need `K` in the low hundreds *for a single drifted prompt*. So a baseline
  captured from **one** run (K = 1 per prompt) has almost no power; a baseline
  aggregated over **many** runs (large K) has real power and a real estimate of
  "normal" variability.
- **Number of prompts.** More prompts means more independent chances to notice a
  regression (balanced by the Šidák correction).

**Design consequence (what ModelSentry does):** the baseline stores a *set* of
answer-embeddings per prompt and capture aggregates the most recent successful
runs. With ≥2 samples per prompt it uses the per-prompt conformal test; with only
one it falls back to the pooled test and *says so*. Power scales smoothly with how
much baseline data you give it — capture more runs to detect subtler drift.

> **Honesty note.** No method conjures power from too little data. With 2 prompts
> and a single baseline run you can only catch *gross* drift, and the tool will
> tell you (large p-values, "pooled" method). The statistics are honest about
> uncertainty; that honesty is the point.

---

<a name="11-pipeline"></a>
## 11. Putting it together: the full pipeline

```
                    ┌──────────────── BASELINE (trusted) ────────────────┐
   For each prompt: │  run the probe K times, embed each completion       │
                    │  → cloud of K answer-embeddings per prompt          │
                    └─────────────────────────────────────────────────────┘
                                          │ stored
                                          ▼
   Each scheduled run:
     1. Send every prompt to the model, get the completion.
     2. Embed each completion → one answer-embedding per prompt.
     3. For each prompt: conformal p-value of the new point vs its baseline cloud.
     4. Combine per-prompt p-values with Šidák → combined_p.
        (If baseline clouds are too small → pooled MMD/energy permutation test.)
     5. Severity from combined_p vs target_fpr.
     6. Alert if combined_p < target_fpr. Fire webhook/Slack with the verdict.
     7. Store the assessment (statistic, p-value, per-prompt breakdown, method).
```

Everything above is calibrated, nonparametric, per-prompt, and reproducible —
which is what makes the result *defensible*, not just *plausible*.

---

<a name="glossary"></a>
## 12. Glossary

- **Alpha (α) / target FPR.** The false-positive rate you choose. Alert when
  `p < α`. Smaller α = fewer false alarms but also less sensitivity.
- **Baseline.** The trusted reference: answer-embeddings captured when the model
  was behaving acceptably. Drift is measured *relative to* the baseline.
- **Centroid.** The average (mean) vector of a set of points; the "center" of a
  cloud.
- **Completion.** The model's answer to a prompt. (Contrast: the prompt is the
  question.) We embed completions, not prompts.
- **Conformal prediction.** A framework for producing valid p-values for "is this
  new point an outlier vs. this reference set?" using ranks, with no
  distributional assumptions.
- **Covariance matrix.** A table describing how each pair of dimensions varies
  together. Needs many samples to estimate; impractical for high-dim embeddings
  with few samples.
- **Dimension.** One coordinate of an embedding vector. Embeddings here have 1536
  or 3072 dimensions.
- **Distribution.** The full, idealized "infinite cloud" of all values a random
  process could produce. We only ever see finite **samples** from it.
- **Drift.** A change over time in the model's behavior (its output
  distribution), relative to the baseline.
- **Embedding.** A fixed-length vector of numbers representing a piece of text,
  arranged so similar meanings give similar vectors.
- **Energy distance.** A parameter-free distance between two distributions; a
  special case of MMD with the negative-distance kernel.
- **Euclidean distance.** Straight-line distance between two points, generalized
  to many dimensions: `√(Σ (xᵢ − yᵢ)²)`.
- **False-positive rate (FPR).** How often you alert when there's actually no
  drift. We set it deliberately via α.
- **Fisher's method.** A way to combine p-values; powerful for many small effects,
  weak for one big effect. We use Šidák instead for alerting.
- **Gaussian / normal distribution.** The classic bell curve. A "Gaussian
  assumption" models data as a bell curve — which we avoid (nonparametric).
- **Kernel.** A similarity function `k(x, y)` between two points. The **RBF
  kernel** is the Gaussian-shaped one we use by default.
- **KL divergence.** A measure of difference between two distributions. The
  closed-form (Gaussian) version needs a covariance matrix and is impractical
  here — hence we use kernel methods instead.
- **LLM (Large Language Model).** A model that generates text (e.g., GPT, Claude).
- **Median heuristic.** Setting the RBF kernel width `σ` to the median of pairwise
  distances in the data — a robust, automatic default.
- **MMD (Maximum Mean Discrepancy).** A kernel-based distance between two
  distributions; zero iff they're identical (with a characteristic kernel).
- **Nonconformity score.** In conformal prediction, a number measuring how
  "strange" a point is relative to a reference set (we use mean distance to the
  cloud).
- **Nonparametric.** Making no assumption about the shape of the distribution
  (no "it's a bell curve"). Robust when data is scarce or weird.
- **Null hypothesis.** The "boring" assumption we test against: *no real drift*.
  P-values measure evidence against it.
- **Permutation test.** Building the no-drift reference distribution by repeatedly
  shuffling group labels and recomputing the statistic. Gives a calibrated
  p-value with no formula assumptions.
- **p-value.** The probability of seeing a statistic at least this extreme if the
  null (no drift) were true. Small ⇒ strong evidence of drift.
- **Power (statistical).** The probability of detecting real drift when it exists.
  Grows with sample size.
- **Probe.** A named, fixed set of prompts sent to one model on a schedule.
- **Sample.** A finite set of observed points drawn from a distribution.
- **Šidák correction.** Adjusting the minimum of `P` p-values via
  `1 − (1 − p_min)^P` to control the false-positive rate across multiple prompts.
- **Statistic.** A single number summarizing the data (here: MMD², energy
  distance, or a combined drift score).
- **Statistical power.** See *Power*.
- **Two-sample test.** A test of whether two samples come from the same
  distribution.
- **Vector.** An ordered list of numbers; a point in multi-dimensional space.

---

<a name="references"></a>
## 13. References and further reading

- A. Gretton, K. Borgwardt, M. Rasch, B. Schölkopf, A. Smola. **"A Kernel
  Two-Sample Test."** *Journal of Machine Learning Research*, 2012. — MMD and the
  permutation test.
- G. Székely, M. Rizzo. **"Energy statistics: A class of statistics based on
  distances."** *Journal of Statistical Planning and Inference*, 2013. — Energy
  distance.
- V. Vovk, A. Gammerman, G. Shafer. **"Algorithmic Learning in a Random World."**
  2005. — Conformal prediction foundations.
- J. Lei, M. G'Sell, A. Rinaldo, R. Tibshirani, L. Wasserman. **"Distribution-Free
  Predictive Inference for Regression."** *JASA*, 2018. — Practical conformal
  methods and validity.
- Z. Šidák. **"Rectangular confidence regions for the means of multivariate normal
  distributions."** *JASA*, 1967. — The Šidák correction.
- B. Efron, R. Tibshirani. **"An Introduction to the Bootstrap."** 1993. —
  Background on resampling / permutation inference.

### On the age of these citations

The dates above are **provenance, not expiry.** Software and model facts age —
`gpt-5.4`, a 1536-dim embedding, an API version. A mathematical proof does not.
Šidák's 1967 result is a *theorem*: it was proved, and a proof is true forever.
The year tells you *when the result was first established*, and scholarly practice
is to cite that primary source, not the newest paper that happens to re-mention
it. "2012" on the MMD paper isn't a freshness stamp — it's where that estimator
was derived.

Three things worth separating:

1. **These results are not superseded.** MMD, energy distance, conformal
   prediction, permutation tests, and Šidák are all still standard practice today.
   Conformal prediction in particular has been one of the most active areas of
   statistics through the 2020s — but the activity *builds on* Vovk/Lei, it
   doesn't invalidate them. There is no newer method that makes them wrong.
2. **They're foundational, not trendy.** The reason to use them isn't recency —
   it's that they have the right *guarantees* for this exact problem
   (distribution-free, finite-sample valid, works when `n ≪ d`). A new paper
   wouldn't change those guarantees; it might add a refinement at the margins.
3. **What genuinely dates in this project is the empirical layer** — provider
   model IDs, embedding dimensions — and that is all config-driven and kept
   current.

So "timeless" is precise here: the math is provably correct independent of the
calendar. If anything newer mattered, it would be a *refinement layered on top*,
not a replacement.

See also, in this repo: [`ARCHITECTURE.md`](ARCHITECTURE.md) for system design,
and the source modules `twosample.rs` and `assessment.rs` for the implementation
with inline derivations.
