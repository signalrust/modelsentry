//! Probe runner: orchestrates one full probe run against a configured provider.
//!
//! [`ProbeRunner::run`] calls the provider for both embeddings and completions.
//! [`ProbeRunner::run_completions_only`] skips embeddings — use this for
//! providers such as Anthropic that have no native embedding endpoint.

use std::sync::Arc;

use chrono::Utc;
use futures::future::join_all;
use tokio::sync::Semaphore;

use modelsentry_common::{
    models::{Probe, ProbeRun, RunStatus},
    types::RunId,
};

use crate::provider::DynProvider;

// ── Public type ───────────────────────────────────────────────────────────────

/// Orchestrates a single probe run — calling the provider with every prompt
/// in the configured [`Probe`] and collecting embeddings + completions.
pub struct ProbeRunner {
    provider: DynProvider,
}

impl ProbeRunner {
    /// Create a new runner backed by `provider`.
    pub fn new(provider: DynProvider) -> Self {
        Self { provider }
    }

    /// Returns `true` if the backing provider supports embeddings.
    #[must_use]
    pub fn has_embeddings(&self) -> bool {
        self.provider.embedding_dim() > 0
    }

    /// Execute all prompts concurrently, sampling each prompt `samples` times and
    /// collecting output embeddings **and** a representative completion.
    ///
    /// At most `concurrency` prompts are in-flight simultaneously (minimum 1
    /// even if `concurrency` is 0); each prompt's `samples` draws run
    /// sequentially within its task. `samples` is clamped to a minimum of 1.
    ///
    /// The returned [`ProbeRun`] always satisfies:
    /// - `embeddings.len() == probe.prompts.len()` (each entry is that prompt's
    ///   list of per-sample output embeddings — possibly fewer than `samples` if
    ///   some draws failed, or empty if all did)
    /// - `completions.len() == probe.prompts.len()` (one representative per prompt)
    ///
    /// A prompt counts as failed (for status classification) when it yields no
    /// usable embedding or no completion. The overall status reflects the failure
    /// ratio: [`RunStatus::Success`], [`RunStatus::PartialFailure`], or
    /// [`RunStatus::Failed`].
    ///
    /// # Errors
    ///
    /// This function does not currently return `Err`.
    pub async fn run(
        &self,
        probe: &Probe,
        concurrency: usize,
        samples: usize,
    ) -> modelsentry_common::error::Result<ProbeRun> {
        let started_at = Utc::now();
        let n_samples = samples.max(1);
        let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));

        let tasks: Vec<_> = probe
            .prompts
            .iter()
            .map(|p| {
                let sem = Arc::clone(&semaphore);
                let prov = Arc::clone(&self.provider);
                let text = p.text.clone();
                async move {
                    let _permit = sem.acquire_owned().await.map_err(|_| {
                        modelsentry_common::error::ModelSentryError::Provider {
                            message: "semaphore closed".to_string(),
                        }
                    })?;
                    // Drift is measured on the model's OUTPUT, so we complete
                    // first and embed the completions — never the (fixed) prompt.
                    // Draw `n_samples` completions to build a within-prompt
                    // distribution, then embed them in a single batched call
                    // (one round-trip instead of one per sample).
                    let mut answers: Vec<String> = Vec::with_capacity(n_samples);
                    for _ in 0..n_samples {
                        let Ok(answer) = prov.complete(&text).await else {
                            continue;
                        };
                        if !answer.is_empty() {
                            answers.push(answer);
                        }
                    }
                    // First usable completion is the representative for display.
                    let representative = answers.first().cloned();
                    let sample_embeddings: Vec<Vec<f32>> = if answers.is_empty() {
                        Vec::new()
                    } else {
                        prov.embed(&answers).await.map_or_else(
                            |_| Vec::new(),
                            |vecs| vecs.iter().map(|e| e.as_slice().to_vec()).collect(),
                        )
                    };
                    Ok::<_, modelsentry_common::error::ModelSentryError>((
                        sample_embeddings,
                        representative,
                    ))
                }
            })
            .collect();

        let outcomes = join_all(tasks).await;

        let n = outcomes.len();
        let mut embeddings = Vec::with_capacity(n);
        let mut completions = Vec::with_capacity(n);
        let mut failure_count: usize = 0;

        for outcome in outcomes {
            let Ok((sample_embeddings, representative)) = outcome else {
                failure_count += 1;
                embeddings.push(Vec::new());
                completions.push(String::new());
                continue;
            };
            // No usable embedding or no completion ⇒ this prompt failed.
            if sample_embeddings.is_empty() || representative.is_none() {
                failure_count += 1;
            }
            embeddings.push(sample_embeddings);
            completions.push(representative.unwrap_or_default());
        }

        Ok(ProbeRun {
            id: RunId::new(),
            probe_id: probe.id.clone(),
            started_at,
            finished_at: Utc::now(),
            embeddings,
            completions,
            drift_report: None,
            status: classify_status(failure_count, n),
        })
    }

    /// Execute all prompts concurrently, collecting **completions only**.
    ///
    /// Use this for providers without embedding support (e.g. Anthropic).
    /// The returned [`ProbeRun`] has all `embeddings` set to empty vectors;
    /// drift metrics that depend on embeddings must be skipped downstream.
    ///
    /// Concurrency semantics and status classification are identical to [`run`].
    ///
    /// [`run`]: ProbeRunner::run
    ///
    /// # Errors
    ///
    /// This function does not currently return `Err`.
    pub async fn run_completions_only(
        &self,
        probe: &Probe,
        concurrency: usize,
    ) -> modelsentry_common::error::Result<ProbeRun> {
        let started_at = Utc::now();
        let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));

        let tasks: Vec<_> = probe
            .prompts
            .iter()
            .map(|p| {
                let sem = Arc::clone(&semaphore);
                let prov = Arc::clone(&self.provider);
                let text = p.text.clone();
                async move {
                    let _permit = sem.acquire_owned().await.map_err(|_| {
                        modelsentry_common::error::ModelSentryError::Provider {
                            message: "semaphore closed".to_string(),
                        }
                    })?;
                    prov.complete(&text).await
                }
            })
            .collect();

        let outcomes = join_all(tasks).await;

        let n = outcomes.len();
        let mut completions = Vec::with_capacity(n);
        let mut failure_count: usize = 0;

        for result in outcomes {
            if let Ok(text) = result {
                completions.push(text);
            } else {
                failure_count += 1;
                completions.push(String::new());
            }
        }

        Ok(ProbeRun {
            id: RunId::new(),
            probe_id: probe.id.clone(),
            started_at,
            finished_at: Utc::now(),
            embeddings: vec![Vec::new(); n],
            completions,
            drift_report: None,
            status: classify_status(failure_count, n),
        })
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn classify_status(failure_count: usize, total: usize) -> RunStatus {
    if failure_count == 0 {
        RunStatus::Success
    } else if failure_count >= total {
        RunStatus::Failed
    } else {
        RunStatus::PartialFailure
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use uuid::Uuid;

    use modelsentry_common::{
        constants::defaults,
        error::{ModelSentryError, Result},
        models::{Probe, ProbePrompt, ProbeSchedule, ProviderSpec, RunStatus},
        types::ProbeId,
    };

    use super::*;
    use crate::drift::Embedding;
    use crate::provider::LlmProvider;

    // ── Fixture ──────────────────────────────────────────────────────────────

    fn make_test_probe(n_prompts: usize) -> Probe {
        Probe {
            id: ProbeId::new(),
            name: "test-probe".into(),
            provider: ProviderSpec::Anthropic {
                model: defaults::anthropic::MODEL.into(),
            },
            prompts: (0..n_prompts)
                .map(|i| ProbePrompt {
                    id: Uuid::new_v4(),
                    text: format!("prompt {i}"),
                    expected_contains: None,
                    expected_not_contains: None,
                })
                .collect(),
            schedule: ProbeSchedule::EveryMinutes { minutes: 5 },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── Test providers ────────────────────────────────────────────────────────

    /// Returns a 2-dim embedding per text and echoes the prompt as completion.
    struct EchoProvider;

    #[async_trait]
    impl LlmProvider for EchoProvider {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>> {
            texts
                .iter()
                .map(|_| Embedding::new(vec![1.0, 2.0]))
                .collect()
        }
        async fn complete(&self, prompt: &str) -> Result<String> {
            Ok(prompt.to_string())
        }
        fn provider_name(&self) -> &'static str {
            "echo"
        }
        fn embedding_dim(&self) -> usize {
            2
        }
    }

    /// Embed always errors; complete succeeds.
    struct FailEmbedProvider;

    #[async_trait]
    impl LlmProvider for FailEmbedProvider {
        async fn embed(&self, _texts: &[String]) -> Result<Vec<Embedding>> {
            Err(ModelSentryError::Provider {
                message: "embed unavailable".into(),
            })
        }
        async fn complete(&self, _prompt: &str) -> Result<String> {
            Ok("ok".into())
        }
        fn provider_name(&self) -> &'static str {
            "fail-embed"
        }
        fn embedding_dim(&self) -> usize {
            0
        }
    }

    /// Tracks peak concurrent embed calls to verify semaphore enforcement.
    struct SlowProvider {
        concurrent: Arc<AtomicUsize>,
        max_concurrent: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LlmProvider for SlowProvider {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>> {
            let current = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_concurrent.fetch_max(current, Ordering::SeqCst);

            tokio::time::sleep(Duration::from_millis(20)).await;

            self.concurrent.fetch_sub(1, Ordering::SeqCst);
            texts.iter().map(|_| Embedding::new(vec![1.0])).collect()
        }
        async fn complete(&self, _prompt: &str) -> Result<String> {
            Ok("ok".into())
        }
        fn provider_name(&self) -> &'static str {
            "slow"
        }
        fn embedding_dim(&self) -> usize {
            1
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_returns_samples_per_prompt() {
        let runner = ProbeRunner::new(Arc::new(EchoProvider));
        let probe = make_test_probe(4);
        let run = runner.run(&probe, 4, 2).await.unwrap();
        assert_eq!(run.embeddings.len(), 4, "one entry per prompt");
        assert!(
            run.embeddings
                .iter()
                .all(|samples| samples.len() == 2 && samples.iter().all(|e| e == &[1.0_f32, 2.0])),
            "each prompt should carry its sampled embeddings",
        );
    }

    #[tokio::test]
    async fn run_returns_one_completion_per_prompt() {
        let runner = ProbeRunner::new(Arc::new(EchoProvider));
        let probe = make_test_probe(3);
        let run = runner.run(&probe, 3, 2).await.unwrap();
        assert_eq!(run.completions.len(), 3);
        for (i, completion) in run.completions.iter().enumerate() {
            assert_eq!(completion, &format!("prompt {i}"));
        }
    }

    #[tokio::test]
    async fn run_failed_embed_propagates_error() {
        let runner = ProbeRunner::new(Arc::new(FailEmbedProvider));
        let probe = make_test_probe(2);
        let run = runner.run(&probe, 2, 2).await.unwrap();
        // all embeds failed — every prompt is counted as failed
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.embeddings.iter().all(Vec::is_empty));
        // completions still populated since complete() succeeded
        assert!(run.completions.iter().all(|c| c == "ok"));
    }

    #[tokio::test]
    async fn run_respects_concurrency_limit() {
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let provider = Arc::new(SlowProvider {
            concurrent: Arc::clone(&concurrent),
            max_concurrent: Arc::clone(&max_concurrent),
        });

        let runner = ProbeRunner::new(provider);
        let probe = make_test_probe(6);
        runner.run(&probe, 2, 1).await.unwrap();

        assert!(
            max_concurrent.load(Ordering::SeqCst) <= 2,
            "expected at most 2 concurrent embed calls, got {}",
            max_concurrent.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn run_id_is_unique_across_two_runs() {
        let runner = ProbeRunner::new(Arc::new(EchoProvider));
        let probe = make_test_probe(1);
        let run_a = runner.run(&probe, 1, 1).await.unwrap();
        let run_b = runner.run(&probe, 1, 1).await.unwrap();
        assert_ne!(run_a.id, run_b.id);
    }

    #[tokio::test]
    async fn run_completions_only_has_empty_embeddings() {
        let runner = ProbeRunner::new(Arc::new(EchoProvider));
        let probe = make_test_probe(3);
        let run = runner.run_completions_only(&probe, 3).await.unwrap();
        assert_eq!(run.embeddings.len(), 3);
        assert!(run.embeddings.iter().all(Vec::is_empty));
        assert_eq!(run.completions.len(), 3);
        assert_eq!(run.status, RunStatus::Success);
    }
}
