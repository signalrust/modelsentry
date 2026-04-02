# Why You Need ModelSentry (Yes, You)

As an expert engineer, you know the ugly truth about LLMs in production: **they silently degrade.**
Providers constantly tweak models behind the scenes. A `gpt-4o` from January is not the same `gpt-4o` in March. Your prompts that worked perfectly start failing, your embeddings drift, and your RAG accuracy slowly bleeds out without a single error log being thrown.

ModelSentry is a self-hosted, un-opinionated daemon built specifically to solve this blind spot. 

## 1. It operates totally out-of-band

You don't need to wrap your application's SDK calls or inject bloated middleware. ModelSentry runs as an independent daemon. You configure a "Probe" (a set of benchmark prompts), point it at your LLM provider, and tell it to run on a cron schedule.
It continually tests the endpoint independently of your user-facing traffic.

## 2. Mathematically rigorous drift detection

It doesn't just check if the API returns a `200 OK`. It captures a baseline of the model's behavior and continuously runs three rigorous statistical checks:
- **KL Divergence:** Detects shifts in the underlying embedding distribution's variance.
- **Cosine Distance:** Detects directional drift in the embedding centroid.
- **Output Entropy:** Measures shifts in vocabulary breadth (is the model suddenly dumbing down its responses?).

---

## 10 Real-World Engineering Nightmares ModelSentry Prevents

Senior developers who have run LLMs in production will recognize these painful, undocumented scenarios. Here is exactly how ModelSentry catches them:

### 1. The "Lazy Model" Compute-Saving Update
**The Nightmare:** To save compute costs during peak hours, your API provider silently tweaks the model. Suddenly, the LLM stops writing full HTML or code files, opting for `// ... rest of the code is unchanged`. Customers complain the generator is broken, but your backend logs show HTTP 200s.
**The Fix:** ModelSentry's **Output Entropy** metric plummets when the model vocabulary narrows. A webhook pages your team instantly about the "lobotomy" before customers ever notice.

### 2. The Unannounced Embedding Precision Shift
**The Nightmare:** You use a vector database for RAG. The provider updates their embedding model's float precision or internal weights. Existing embeddings in your PGVector database no longer geometrically align with newly generated embeddings. Search relevance slowly decays.
**The Fix:** ModelSentry freezes a baseline snapshot of your embedding centroid. Any underlying change to the latent space triggers a massive spike in **KL Divergence**, warning you to rebuild your vector index immediately.

### 3. The "Over-Apologetic" RLHF Alignment Update
**The Nightmare:** The provider pushes a safety alignment update (RLHF) to make the model "safer." Suddenly, your automated medical document parser or legal contract analyzer is getting responses like, *"As an AI, I cannot give medical advice..."* instead of extracting the JSON payload you requested.
**The Fix:** The semantic meaning of the output shifts from "factual data" to "apologetic refusal". ModelSentry catches a massive drift in **Cosine Distance** on your domain-specific probe prompts.

### 4. The Accidental Local Quantization Swap
**The Nightmare:** Your DevOps engineer is migrating your local `Ollama` cluster and accidentally pulls a `q4_0` (4-bit quantization) model variant instead of the production `q8_0` (8-bit) variant. The app "works", but the model hallucinates wildly and forgets edge-case context.
**The Fix:** The degraded precision of a smaller quantization alters the embedding variance. ModelSentry’s scheduled probes detect an immediate baseline violation in **KL Divergence** on your localhost endpoint.

### 5. Chatty JSON Schema Decay
**The Nightmare:** Your pipeline strictly relies on `JSON.parse()`. After weeks of stability, the model suddenly starts prepending *"Sure, here is the JSON you requested:"* before the payload, entirely shattering your parsing layer.
**The Fix:** ModelSentry's baseline expects a specific statistical distribution of payload tokens. Conversational filler radically alters the **Output Entropy**, triggering a "High Drift" alert.

### 6. The LLM Gateway "Bait and Switch"
**The Nightmare:** You transition to an LLM router/gateway that promises "GPT-4 level quality at half the price." A month in, you suspect they are secretly routing 15% of your expensive queries to cheaper, smaller models (like `Llama-3-8B`) to pocket the margin.
**The Fix:** ModelSentry probes the gateway on a cron job. When cheaper models handle the requests, the variance of the responses spikes. ModelSentry detects bimodal variance via **KL Divergence**, letting you catch the proxy provider red-handed.

### 7. Multi-Lingual Vocabulary Forgetting
**The Nightmare:** Your app is localized for European customers. A new base model update drastically improves English performance, but regresses heavily for French and German, causing the model to default to simplistic, middle-school-level grammar.
**The Fix:** You set up a French-language probe. When the model loses its complex linguistic weights, the **Output Entropy** on non-English probes drops significantly, alerting your engineering team to avoid the model bump.

### 8. System-Wide Temperature Misconfiguration 
**The Nightmare:** A junior engineer accidentally modifies the default `temperature` in your shared Azure config from `0.0` (highly deterministic) to `0.8` (highly creative). A financial data extraction pipeline relying on identical reproducibility turns into a random number generator.
**The Fix:** Higher temperature flattens probability curves. ModelSentry's captured baseline (at `temp=0.0`) will wildly mismatch the new highly-variable outputs, instantly throwing a **KL Divergence** alert.

### 9. Prompt Injection Regression
**The Nightmare:** You spent weeks crafting a system prompt that protects against "ignore all previous instructions" injection attacks. The base model updates, and suddenly the LLM becomes much more compliant to user overrides.
**The Fix:** You add known injection payloads to your ModelSentry probe. If the model is robust, it returns standard rejection embeddings. If it regresses and complies, the semantic meaning changes entirely, caught by **Cosine Distance** drift.

### 10. Provider Gaslighting During Outages
**The Nightmare:** Your app's AI features feel degraded. You open a support ticket with your Enterprise provider, and they tell you: *"We haven't changed anything on our end, check your own prompts."*
**The Fix:** ModelSentry retains immutable, chronological `DriftReports` mapped to an embedded `redb` database. You simply export the chart proving that embedding centroids shifted by 15% starting exactly at 2:14 PM UTC on Tuesday. Mathematical proof ends the argument instantly.

---

## 3. Paranoid about security and privacy

Because ModelSentry runs entirely on your own infrastructure:
- **Zero data exfiltration:** Your proprietary prompts and probe results never leave your VPC. No third-party SaaS dashboard gets to snoop on your LLM inputs.
- **Encrypted secrets:** API keys are AES-GCM encrypted via `age` at rest in a local vault. The Rust daemon uses the `secrecy` crate to ensure keys are zeroized and never accidentally dumped to standard out or logs.

## 4. Uncompromising Engineering Standards

This isn't a hacky python script. It's written in deeply optimized, `#![forbid(unsafe_code)]` Rust. 
- The persistence layer is `redb` (a pure-Rust embedded db), meaning zero external dependencies — no Postgres or Redis to manage. 
- Memory footprint is tiny.
- Highly concurrent scheduler built on `tokio` running probes in parallel.

## 5. Instant Webhook Alerts

The moment a model is lobotomized or quantized in a way that breaks your thresholds, ModelSentry catches the drift and fires a webhook to Slack/PagerDuty before your customers even notice the degradation.

**TL;DR:** ModelSentry gives you deterministic monitoring for non-deterministic AI. Run it locally via `docker compose up`, add a probe, and get back your peace of mind.
