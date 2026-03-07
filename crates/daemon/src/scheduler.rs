//! Probe scheduler — loads all [`Probe`]s from the store and fires a
//! [`ProbeRunner`] for each one on the configured [`ProbeSchedule`].
//!
//! Entry point: [`Scheduler::start`], which returns a [`SchedulerHandle`] that
//! can be used to stop the scheduler gracefully.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::Utc;
use cron::Schedule;

use modelsentry_common::error::Result;
use modelsentry_common::models::{Probe, ProbeSchedule, ProviderKind};
use modelsentry_core::alert::AlertEngine;
use modelsentry_core::drift::calculator::DriftCalculator;
use modelsentry_core::probe_runner::ProbeRunner;
use modelsentry_core::provider::DynProvider;
use modelsentry_store::AppStore;
use tokio::sync::oneshot;
use tokio::task::JoinSet;

/// Maps a provider identifier (e.g. `"anthropic"`, `"openai"`) to a
/// [`DynProvider`] implementation.
pub type ProviderRegistry = RwLock<HashMap<String, DynProvider>>;

/// Convenience constructor for an empty [`ProviderRegistry`].
#[must_use]
pub fn new_registry() -> ProviderRegistry {
    RwLock::new(HashMap::new())
}

/// Tokio-based scheduler that fires [`ProbeRunner`] for each probe on its
/// configured schedule and writes results back to the store.
pub struct Scheduler {
    store: Arc<AppStore>,
    providers: Arc<ProviderRegistry>,
    calculator: Arc<DriftCalculator>,
    alert_engine: Arc<AlertEngine>,
}

/// Handle returned by [`Scheduler::start`].
///
/// Call [`SchedulerHandle::shutdown`] to stop the scheduler and wait for all
/// probe-loop tasks to abort.
pub struct SchedulerHandle {
    shutdown_tx: oneshot::Sender<()>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl Scheduler {
    /// Create a new scheduler with the given components.
    #[must_use]
    pub fn new(
        store: Arc<AppStore>,
        providers: Arc<ProviderRegistry>,
        calculator: Arc<DriftCalculator>,
        alert_engine: Arc<AlertEngine>,
    ) -> Self {
        Self {
            store,
            providers,
            calculator,
            alert_engine,
        }
    }

    /// Start the scheduler. Returns a [`SchedulerHandle`]; call
    /// [`SchedulerHandle::shutdown`] to stop it.
    ///
    /// # Errors
    ///
    /// Returns an error if the probe list cannot be loaded from the store on
    /// startup.
    #[must_use]
    pub fn start(self) -> SchedulerHandle {
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let store = Arc::clone(&self.store);
        let providers = Arc::clone(&self.providers);
        let calculator = Arc::clone(&self.calculator);
        let alert_engine = Arc::clone(&self.alert_engine);

        let join_handle = tokio::spawn(async move {
            let probes = match store.probes().list_all() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("scheduler: failed to load probes on startup: {e}");
                    return;
                }
            };

            let mut set = JoinSet::new();
            for probe in probes {
                let store = Arc::clone(&store);
                let providers = Arc::clone(&providers);
                let calculator = Arc::clone(&calculator);
                let alert_engine = Arc::clone(&alert_engine);
                set.spawn(run_probe_loop(
                    probe,
                    store,
                    providers,
                    calculator,
                    alert_engine,
                ));
            }

            tokio::select! {
                _ = &mut shutdown_rx => {
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                }
            }
        });

        SchedulerHandle {
            shutdown_tx,
            join_handle,
        }
    }
}

impl SchedulerHandle {
    /// Shut down the scheduler, aborting all probe-loop tasks and joining the
    /// main scheduler task.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.join_handle.await;
    }
}

// ---------------------------------------------------------------------------
// Internal — probe loop and job execution
// ---------------------------------------------------------------------------

async fn run_probe_loop(
    probe: Probe,
    store: Arc<AppStore>,
    providers: Arc<ProviderRegistry>,
    calculator: Arc<DriftCalculator>,
    alert_engine: Arc<AlertEngine>,
) {
    loop {
        tokio::time::sleep(sleep_until_next_run(&probe.schedule)).await;

        let key = provider_key(&probe.provider);
        let provider = match providers.read() {
            Ok(guard) => guard.get(&key).cloned(),
            Err(poisoned) => {
                tracing::error!(
                    probe_id = %probe.id,
                    "scheduler: provider registry RwLock poisoned: {poisoned}",
                );
                continue;
            }
        };
        let Some(provider) = provider else {
            tracing::error!(
                probe_id = %probe.id,
                "scheduler: no provider registered for '{key}' — skipping run",
            );
            continue;
        };

        if let Err(e) = run_probe_job(&probe, &store, provider, &calculator, &alert_engine).await {
            tracing::error!(probe_id = %probe.id, "scheduler: probe run failed: {e}");
        }
    }
}

async fn run_probe_job(
    probe: &Probe,
    store: &AppStore,
    provider: DynProvider,
    calculator: &DriftCalculator,
    alert_engine: &AlertEngine,
) -> Result<()> {
    let has_embeddings = provider.embedding_dim() > 0;
    let runner = ProbeRunner::new(provider);
    let concurrency = 4;

    let mut run = if has_embeddings {
        runner.run(probe, concurrency).await?
    } else {
        runner.run_completions_only(probe, concurrency).await?
    };

    // Drift detection — only when we have embeddings and a baseline exists.
    if !run.embeddings.is_empty() {
        if let Some(baseline) = store.baselines().get_latest_for_probe(&probe.id)? {
            if let Ok(report) = calculator.compute(&run, &baseline) {
                let rules = store.alerts().get_rules_for_probe(&probe.id)?;
                let events = alert_engine.evaluate_and_fire(&report, &rules).await;
                for event in &events {
                    store.alerts().insert_event(event)?;
                }
                run.drift_report = Some(report);
            }
        }
    }

    store.runs().insert(&run)?;
    Ok(())
}

fn sleep_until_next_run(schedule: &ProbeSchedule) -> Duration {
    match schedule {
        ProbeSchedule::EveryMinutes { minutes } => Duration::from_secs(u64::from(*minutes) * 60),
        ProbeSchedule::Cron { expression } => {
            // Support both 6-field (sec min hour dom month dow) and standard
            // 5-field (min hour dom month dow) cron by auto-prepending "0 ".
            let parsed = Schedule::from_str(expression)
                .or_else(|_| Schedule::from_str(&format!("0 {expression}")));
            match parsed {
                Ok(sched) => sched
                    .upcoming(Utc)
                    .next()
                    .and_then(|t| (t - Utc::now()).to_std().ok())
                    .unwrap_or(Duration::from_secs(1)),
                Err(e) => {
                    tracing::warn!(
                        expr = expression,
                        "invalid cron expression: {e}; falling back to 1-hour interval"
                    );
                    Duration::from_secs(60 * 60)
                }
            }
        }
    }
}

fn provider_key(kind: &ProviderKind) -> String {
    match kind {
        ProviderKind::OpenAi => "openai".to_string(),
        ProviderKind::Anthropic => "anthropic".to_string(),
        ProviderKind::Ollama { base_url } => format!("ollama:{base_url}"),
        ProviderKind::AzureOpenAi {
            endpoint,
            deployment,
        } => format!("azure:{endpoint}:{deployment}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use modelsentry_common::{
        error::{ModelSentryError, Result},
        models::{Probe, ProbePrompt, ProbeSchedule, ProviderKind},
        types::ProbeId,
    };
    use modelsentry_core::{
        alert::AlertEngine,
        drift::{Embedding, calculator::DriftCalculator},
        provider::LlmProvider,
    };
    use modelsentry_store::AppStore;
    use tempfile::TempDir;
    use uuid::Uuid;

    // --- test providers ---

    /// Provider that always returns a successful completion and no embeddings.
    struct OkProvider;

    #[async_trait]
    impl LlmProvider for OkProvider {
        async fn embed(&self, _: &[String]) -> Result<Vec<Embedding>> {
            Err(ModelSentryError::Provider {
                message: "not supported".to_string(),
            })
        }
        async fn complete(&self, _: &str) -> Result<String> {
            Ok("pong".to_string())
        }
        fn provider_name(&self) -> &'static str {
            "test"
        }
        fn embedding_dim(&self) -> usize {
            0
        }
    }

    /// Provider that always fails.
    struct FailProvider;

    #[async_trait]
    impl LlmProvider for FailProvider {
        async fn embed(&self, _: &[String]) -> Result<Vec<Embedding>> {
            Err(ModelSentryError::Provider {
                message: "fail".to_string(),
            })
        }
        async fn complete(&self, _: &str) -> Result<String> {
            Err(ModelSentryError::Provider {
                message: "fail".to_string(),
            })
        }
        fn provider_name(&self) -> &'static str {
            "test"
        }
        fn embedding_dim(&self) -> usize {
            0
        }
    }

    // --- helpers ---

    fn open_store() -> (TempDir, Arc<AppStore>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let store = Arc::new(AppStore::open(&path).unwrap());
        (dir, store)
    }

    fn make_probe(minutes: u32) -> Probe {
        Probe {
            id: ProbeId::new(),
            name: "test-probe".to_string(),
            provider: ProviderKind::Anthropic,
            model: "test-model".to_string(),
            prompts: vec![ProbePrompt {
                id: Uuid::new_v4(),
                text: "ping?".to_string(),
                expected_contains: None,
                expected_not_contains: None,
            }],
            schedule: ProbeSchedule::EveryMinutes { minutes },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_scheduler(store: Arc<AppStore>, provider: Arc<dyn LlmProvider>) -> Scheduler {
        let registry = Arc::new(new_registry());
        registry
            .write()
            .unwrap()
            .insert("anthropic".to_string(), provider);
        Scheduler::new(
            store,
            registry,
            Arc::new(DriftCalculator::new(0.5, 0.5).unwrap()),
            Arc::new(AlertEngine::default()),
        )
    }

    // --- tests ---

    /// Verify that after one interval elapses, a run is written to the store.
    #[tokio::test(start_paused = true)]
    async fn scheduler_runs_probe_on_tick() {
        let (_dir, store) = open_store();
        let probe = make_probe(5);
        let probe_id = probe.id.clone();
        store.probes().insert(&probe).unwrap();

        let handle = make_scheduler(Arc::clone(&store), Arc::new(OkProvider)).start();

        // Let outer task spawn probe-loop tasks and let them register sleep futures.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(6 * 60)).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        let runs = store.runs().list_for_probe(&probe_id, 10).unwrap();
        assert_eq!(runs.len(), 1, "expected exactly one run after one tick");

        handle.shutdown().await;
    }

    /// Verify that the run written to the store has the expected `probe_id` and
    /// that completions were captured.
    #[tokio::test(start_paused = true)]
    async fn scheduler_writes_run_to_store() {
        let (_dir, store) = open_store();
        let probe = make_probe(1);
        let probe_id = probe.id.clone();
        store.probes().insert(&probe).unwrap();

        let handle = make_scheduler(Arc::clone(&store), Arc::new(OkProvider)).start();

        // Let outer task spawn probe-loop tasks and let them register sleep futures.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(90)).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        let run = store
            .runs()
            .list_for_probe(&probe_id, 1)
            .unwrap()
            .into_iter()
            .next()
            .expect("expected a run in the store");

        assert_eq!(run.probe_id, probe_id);
        assert_eq!(run.completions.len(), 1, "one completion per prompt");

        handle.shutdown().await;
    }

    /// Shutdown must complete without hanging, even when no probes are loaded.
    #[tokio::test]
    async fn scheduler_shuts_down_cleanly() {
        let (_dir, store) = open_store();
        let handle = Scheduler::new(
            store,
            Arc::new(new_registry()),
            Arc::new(DriftCalculator::new(1.0, 1.0).unwrap()),
            Arc::new(AlertEngine::default()),
        )
        .start();

        handle.shutdown().await; // must not hang or panic
    }

    /// A probe run that fails must log an error but must not crash the
    /// scheduler loop — the handle must still be shutdownable.
    #[tokio::test(start_paused = true)]
    async fn probe_run_failure_does_not_crash_scheduler() {
        let (_dir, store) = open_store();
        let probe = make_probe(5);
        store.probes().insert(&probe).unwrap();

        let handle = make_scheduler(Arc::clone(&store), Arc::new(FailProvider)).start();

        // Let outer task spawn probe-loop tasks and let them register sleep futures.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(6 * 60)).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        // Scheduler must still be alive — shutdown must complete.
        handle.shutdown().await;
    }

    // --- cron schedule unit tests ---

    #[test]
    fn every_minutes_gives_exact_duration() {
        let s = ProbeSchedule::EveryMinutes { minutes: 15 };
        assert_eq!(sleep_until_next_run(&s), Duration::from_secs(15 * 60));
    }

    #[test]
    fn valid_cron_gives_positive_duration_under_one_period() {
        // "every minute" — next tick is ≤60 s away
        let s = ProbeSchedule::Cron {
            expression: "* * * * *".to_string(),
        };
        let dur = sleep_until_next_run(&s);
        assert!(
            dur <= Duration::from_secs(60),
            "expected ≤60 s, got {dur:?}"
        );
    }

    #[test]
    fn invalid_cron_falls_back_to_hourly() {
        let s = ProbeSchedule::Cron {
            expression: "not-a-cron-expr".to_string(),
        };
        assert_eq!(sleep_until_next_run(&s), Duration::from_secs(60 * 60));
    }

    #[test]
    fn six_field_cron_is_parsed_directly() {
        // 6-field: sec min hour dom month dow — "every minute at second 0"
        let s = ProbeSchedule::Cron {
            expression: "0 * * * * *".to_string(),
        };
        let dur = sleep_until_next_run(&s);
        assert!(
            dur <= Duration::from_secs(60),
            "expected ≤60 s, got {dur:?}"
        );
    }
}
