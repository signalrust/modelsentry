//! Probe scheduler — loads all [`Probe`]s from the store and fires a
//! [`ProbeRunner`] for each one on the configured [`ProbeSchedule`].
//!
//! Entry point: [`Scheduler::start`], which returns a [`SchedulerHandle`] that
//! can be used to stop the scheduler gracefully.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cron::Schedule;

use modelsentry_common::error::{ModelSentryError, Result};
use modelsentry_common::models::{Probe, ProbeSchedule};
use modelsentry_common::types::ProbeId;
use modelsentry_core::alert::AlertEngine;
use modelsentry_core::drift::calculator::DriftCalculator;
use modelsentry_core::probe_runner::ProbeRunner;
use modelsentry_core::provider::DynProvider;
use modelsentry_store::AppStore;
use tokio::sync::{Semaphore, oneshot};
use tokio::task::JoinHandle;

use crate::constants::runtime::{PROBE_CONCURRENCY, RECONCILE_INTERVAL};
use crate::provider_factory::ProviderResolver;

/// Tokio-based scheduler that fires [`ProbeRunner`] for each probe on its
/// configured schedule and writes results back to the store.
///
/// Providers are resolved per run from the probe's
/// [`ProviderSpec`](modelsentry_common::models::ProviderSpec) via the injected
/// [`ProviderResolver`] — there is no provider registry, so a key added to the
/// vault takes effect on the next run with no restart.
pub struct Scheduler {
    store: Arc<AppStore>,
    resolver: Arc<dyn ProviderResolver>,
    calculator: Arc<DriftCalculator>,
    alert_engine: Arc<AlertEngine>,
    /// Completions sampled per prompt per run (drift resolution vs. cost).
    samples_per_prompt: usize,
    /// Caps probe runs in flight across all probes, so a fleet (or a restart
    /// that finds several probes overdue) cannot stampede a provider.
    run_semaphore: Arc<Semaphore>,
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
    ///
    /// `max_concurrent_runs` bounds probe runs in flight across all probes
    /// (clamped to a minimum of 1).
    #[must_use]
    pub fn new(
        store: Arc<AppStore>,
        resolver: Arc<dyn ProviderResolver>,
        calculator: Arc<DriftCalculator>,
        alert_engine: Arc<AlertEngine>,
        samples_per_prompt: usize,
        max_concurrent_runs: usize,
    ) -> Self {
        Self {
            store,
            resolver,
            calculator,
            alert_engine,
            samples_per_prompt,
            run_semaphore: Arc::new(Semaphore::new(max_concurrent_runs.max(1))),
        }
    }

    /// Start the scheduler. Returns a [`SchedulerHandle`]; call
    /// [`SchedulerHandle::shutdown`] to stop it.
    ///
    /// The scheduler periodically reconciles its running per-probe loops against
    /// the store (every [`RECONCILE_INTERVAL`]), so probes added, edited, or
    /// deleted via the API/CLI after startup take effect without a restart.
    #[must_use]
    pub fn start(self) -> SchedulerHandle {
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let store = Arc::clone(&self.store);
        let resolver = Arc::clone(&self.resolver);
        let calculator = Arc::clone(&self.calculator);
        let alert_engine = Arc::clone(&self.alert_engine);
        let samples_per_prompt = self.samples_per_prompt;
        let run_semaphore = Arc::clone(&self.run_semaphore);

        let join_handle = tokio::spawn(async move {
            // Currently-scheduled probe loops, keyed by probe id.
            let mut active: HashMap<ProbeId, ActiveProbe> = HashMap::new();
            let mut ticker = tokio::time::interval(RECONCILE_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        for (_, ap) in active.drain() {
                            ap.handle.abort();
                        }
                        break;
                    }
                    _ = ticker.tick() => {
                        reconcile(
                            &store,
                            &resolver,
                            &calculator,
                            &alert_engine,
                            samples_per_prompt,
                            &run_semaphore,
                            &mut active,
                        );
                    }
                }
            }
        });

        SchedulerHandle {
            shutdown_tx,
            join_handle,
        }
    }
}

/// A per-probe loop the scheduler is currently running, tracked so it can be
/// cancelled (probe deleted) or replaced (probe edited).
struct ActiveProbe {
    /// The probe's `updated_at` when this loop was spawned; a change means the
    /// probe was edited and the loop must be respawned with the new definition.
    updated_at: DateTime<Utc>,
    handle: JoinHandle<()>,
}

/// Reconcile the running per-probe loops against the current store contents:
/// stop loops for deleted probes, (re)spawn loops for new or edited probes, and
/// leave unchanged probes running.
fn reconcile(
    store: &Arc<AppStore>,
    resolver: &Arc<dyn ProviderResolver>,
    calculator: &Arc<DriftCalculator>,
    alert_engine: &Arc<AlertEngine>,
    samples_per_prompt: usize,
    run_semaphore: &Arc<Semaphore>,
    active: &mut HashMap<ProbeId, ActiveProbe>,
) {
    let probes = match store.probes().list_all() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("scheduler: failed to list probes during reconcile: {e}");
            return;
        }
    };

    let current: HashSet<ProbeId> = probes.iter().map(|p| p.id.clone()).collect();

    // Stop loops whose probe was deleted.
    active.retain(|id, ap| {
        if current.contains(id) {
            true
        } else {
            tracing::info!(probe_id = %id, "scheduler: probe removed — stopping its loop");
            ap.handle.abort();
            false
        }
    });

    // Spawn loops for new probes; respawn for edited ones (schedule may differ).
    for probe in probes {
        let needs_spawn = active
            .get(&probe.id)
            .is_none_or(|ap| ap.updated_at != probe.updated_at);
        if !needs_spawn {
            continue;
        }
        if let Some(old) = active.remove(&probe.id) {
            old.handle.abort();
            // The schedule may have changed; drop the persisted next-run so the
            // new schedule takes effect now instead of after the pending run.
            if let Err(e) = store.schedule().delete(&probe.id) {
                tracing::warn!(probe_id = %probe.id, "scheduler: could not reset schedule state: {e}");
            }
            tracing::info!(probe_id = %probe.id, "scheduler: probe changed — rescheduling");
        } else {
            tracing::info!(probe_id = %probe.id, "scheduler: probe added — scheduling");
        }
        let id = probe.id.clone();
        let updated_at = probe.updated_at;
        let handle = tokio::spawn(run_probe_loop(
            probe,
            Arc::clone(store),
            Arc::clone(resolver),
            Arc::clone(calculator),
            Arc::clone(alert_engine),
            samples_per_prompt,
            Arc::clone(run_semaphore),
        ));
        active.insert(id, ActiveProbe { updated_at, handle });
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
    resolver: Arc<dyn ProviderResolver>,
    calculator: Arc<DriftCalculator>,
    alert_engine: Arc<AlertEngine>,
    samples_per_prompt: usize,
    run_semaphore: Arc<Semaphore>,
) {
    loop {
        // Sleep until this probe's persisted next-run time. On a fresh start
        // this is "one interval from now"; after a restart it is the time stored
        // before shutdown — already in the past for an overdue probe, so the
        // sleep is zero and the missed run fires immediately (single catch-up).
        let next_at = scheduled_next_run(&store, &probe);
        let wait = (next_at - Utc::now()).to_std().unwrap_or(Duration::ZERO);
        tokio::time::sleep(wait).await;

        // Resolve the provider fresh each run so vault/config changes apply
        // without a restart, and a misconfigured probe never aborts the loop.
        let provider = match resolver.resolve(&probe.provider) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(
                    probe_id = %probe.id,
                    "scheduler: cannot build provider for probe — skipping run: {e}",
                );
                // Advance the schedule so a persistently-bad provider does not
                // hot-loop on an already-due next-run time.
                advance_schedule(&store, &probe);
                continue;
            }
        };

        // Bound the number of probe runs in flight across all probes. The permit
        // is held only for the duration of the run (released at the block end). A
        // closed semaphore means the scheduler is shutting down, so exit.
        {
            let Ok(_permit) = run_semaphore.clone().acquire_owned().await else {
                return;
            };
            if let Err(e) = run_probe_job(
                &probe,
                &store,
                provider,
                &calculator,
                &alert_engine,
                samples_per_prompt,
            )
            .await
            {
                tracing::error!(probe_id = %probe.id, "scheduler: probe run failed: {e}");
            }
        }

        advance_schedule(&store, &probe);
    }
}

/// Read the probe's next scheduled run from the store, initializing and
/// persisting it for a brand-new probe (no stored state). Falls back to a fresh
/// interval if the state cannot be read.
fn scheduled_next_run(store: &AppStore, probe: &Probe) -> DateTime<Utc> {
    match store.schedule().get_next_run(&probe.id) {
        Ok(Some(next_at)) => next_at,
        Ok(None) => {
            let next_at = next_run_after(&probe.schedule, Utc::now());
            advance_schedule_to(store, &probe.id, next_at);
            next_at
        }
        Err(e) => {
            tracing::warn!(
                probe_id = %probe.id,
                "scheduler: cannot read schedule state, falling back to interval: {e}",
            );
            next_run_after(&probe.schedule, Utc::now())
        }
    }
}

/// Persist the *following* run time (one schedule step after now) for `probe`.
fn advance_schedule(store: &AppStore, probe: &Probe) {
    advance_schedule_to(
        store,
        &probe.id,
        next_run_after(&probe.schedule, Utc::now()),
    );
}

/// Persist `next_at` as `probe_id`'s next-run time, logging on failure (a failed
/// write only loses restart-resume precision for this probe, never a run).
fn advance_schedule_to(store: &AppStore, probe_id: &ProbeId, next_at: DateTime<Utc>) {
    if let Err(e) = store.schedule().set_next_run(probe_id, next_at) {
        tracing::warn!(probe_id = %probe_id, "scheduler: failed to persist next-run time: {e}");
    }
}

async fn run_probe_job(
    probe: &Probe,
    store: &AppStore,
    provider: DynProvider,
    calculator: &DriftCalculator,
    alert_engine: &AlertEngine,
    samples_per_prompt: usize,
) -> Result<()> {
    let has_embeddings = provider.embedding_dim() > 0;
    let runner = ProbeRunner::new(provider);
    let concurrency = PROBE_CONCURRENCY;

    let mut run = if has_embeddings {
        runner.run(probe, concurrency, samples_per_prompt).await?
    } else {
        runner.run_completions_only(probe, concurrency).await?
    };

    // Drift detection — only when the run produced at least one output embedding
    // and a baseline exists.
    let has_output_embeddings = run
        .embeddings
        .iter()
        .any(|samples| samples.iter().any(|e| !e.is_empty()));
    if has_output_embeddings {
        if let Some(baseline) = store.baselines().get_latest_for_probe(&probe.id)? {
            match calculator.compute(&run, &baseline) {
                Ok(report) => {
                    let rules = store.alerts().get_rules_for_probe(&probe.id)?;
                    // Load each rule's last-fired time so the engine can apply
                    // the cooldown / de-duplication window.
                    let mut last_fired = HashMap::new();
                    for rule in &rules {
                        if let Some(at) = store.alerts().last_fired_for_rule(&rule.id)? {
                            last_fired.insert(rule.id.clone(), at);
                        }
                    }
                    // Sequential control: load each rule's alpha already spent
                    // inside the current window, so the engine can cap this
                    // run's testing level against the remaining budget.
                    let mut spent_alpha = HashMap::new();
                    if let Some(window) = alert_engine.sequential_window() {
                        let since = Utc::now() - window;
                        for rule in &rules {
                            let spent = store.spends().spent_since(&rule.id, since)?;
                            if spent > 0.0 {
                                spent_alpha.insert(rule.id.clone(), spent);
                            }
                        }
                    }
                    let outcome = alert_engine
                        .evaluate_and_fire(&report, &rules, &last_fired, &spent_alpha)
                        .await;
                    for event in &outcome.events {
                        store.alerts().insert_event(event)?;
                    }
                    // Persist this run's alpha debits so the budget spans runs
                    // and restarts; prune spends that have aged out of the
                    // window to keep the ledger bounded.
                    if let Some(window) = alert_engine.sequential_window() {
                        let now = Utc::now();
                        let prune_before = now - window;
                        for spend in &outcome.spends {
                            store.spends().record_spend(
                                &spend.rule_id,
                                now,
                                spend.alpha,
                                prune_before,
                            )?;
                        }
                    }
                    run.drift_report = Some(report);
                }
                // The embedding model changed since the baseline was captured —
                // surface it loudly so the operator re-captures, rather than
                // silently recording runs with no drift report.
                Err(ModelSentryError::BaselineEmbeddingMismatch {
                    baseline_dim,
                    run_dim,
                }) => {
                    tracing::warn!(
                        probe_id = %probe.id,
                        baseline_dim,
                        run_dim,
                        "scheduler: embedding dimension changed since baseline capture — \
                         re-capture the baseline for this probe to resume drift detection",
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        probe_id = %probe.id,
                        error = %e,
                        "scheduler: drift computation skipped",
                    );
                }
            }
        }
    }

    store.runs().insert(&run)?;
    Ok(())
}

/// Parse a probe cron expression, accepting both 6-field
/// (`sec min hour dom month dow`) and standard 5-field (`min hour dom month dow`)
/// forms — the latter by auto-prepending `"0 "`.
///
/// # Errors
///
/// Returns a human-readable message if the expression parses as neither form.
pub fn parse_cron_schedule(expression: &str) -> std::result::Result<Schedule, String> {
    Schedule::from_str(expression)
        .or_else(|_| Schedule::from_str(&format!("0 {expression}")))
        .map_err(|e| format!("invalid cron expression '{expression}': {e}"))
}

/// Validate a [`ProbeSchedule`] before it is persisted, so a bad schedule is
/// rejected at probe-create time instead of silently degrading at runtime.
///
/// # Errors
///
/// - `EveryMinutes { minutes: 0 }` would busy-loop the scheduler.
/// - An unparseable cron expression would otherwise fall back to hourly.
pub fn validate_schedule(schedule: &ProbeSchedule) -> std::result::Result<(), String> {
    match schedule {
        ProbeSchedule::EveryMinutes { minutes } => {
            if *minutes == 0 {
                return Err("schedule.minutes must be at least 1".to_string());
            }
            Ok(())
        }
        ProbeSchedule::Cron { expression } => parse_cron_schedule(expression).map(|_| ()),
    }
}

/// The next absolute run time for `schedule`, computed relative to `from`.
///
/// For `EveryMinutes` this is `from + minutes` (a 0-minute schedule that slipped
/// past validation is floored to 1 minute, never a busy loop). For `Cron` it is
/// the next firing strictly after `from`; an unparseable expression that somehow
/// reached the scheduler falls back to one hour out.
fn next_run_after(schedule: &ProbeSchedule, from: DateTime<Utc>) -> DateTime<Utc> {
    match schedule {
        ProbeSchedule::EveryMinutes { minutes } => {
            from + chrono::Duration::minutes(i64::from((*minutes).max(1)))
        }
        ProbeSchedule::Cron { expression } => match parse_cron_schedule(expression) {
            Ok(sched) => sched
                .after(&from)
                .next()
                .unwrap_or_else(|| from + chrono::Duration::hours(1)),
            Err(e) => {
                tracing::warn!(
                    expr = expression,
                    "{e}; falling back to 1-hour interval (this probe should have been \
                     rejected at create time)"
                );
                from + chrono::Duration::hours(1)
            }
        },
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
        constants::defaults,
        error::{ModelSentryError, Result},
        models::{Probe, ProbePrompt, ProbeSchedule, ProviderSpec},
        types::ProbeId,
    };
    use modelsentry_core::{
        alert::AlertEngine,
        drift::{Embedding, assessment::AssessmentConfig, calculator::DriftCalculator},
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

    /// Provider that returns a fixed 4-dim embedding and a constant completion,
    /// so the drift-detection branch of `run_probe_job` is exercised.
    struct EmbedProvider;

    #[async_trait]
    impl LlmProvider for EmbedProvider {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>> {
            texts
                .iter()
                .map(|_| Embedding::new(vec![0.05, 0.0, 0.0, 0.0]))
                .collect()
        }
        async fn complete(&self, _: &str) -> Result<String> {
            Ok("answer".to_string())
        }
        fn provider_name(&self) -> &'static str {
            "test"
        }
        fn embedding_dim(&self) -> usize {
            4
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
            provider: ProviderSpec::Anthropic {
                model: defaults::anthropic::MODEL.to_string(),
            },
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

    /// Test resolver that hands out a fixed stub provider for any spec, so the
    /// scheduler loop can be exercised without a vault or network.
    struct StubResolver(Arc<dyn LlmProvider>);

    impl crate::provider_factory::ProviderResolver for StubResolver {
        fn resolve(&self, _spec: &ProviderSpec) -> Result<DynProvider> {
            Ok(Arc::clone(&self.0))
        }
    }

    fn make_scheduler(store: Arc<AppStore>, provider: Arc<dyn LlmProvider>) -> Scheduler {
        Scheduler::new(
            store,
            Arc::new(StubResolver(provider)),
            Arc::new(DriftCalculator::new(AssessmentConfig::default())),
            Arc::new(AlertEngine::default()),
            1,
            4,
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
            Arc::new(StubResolver(Arc::new(OkProvider))),
            Arc::new(DriftCalculator::new(AssessmentConfig::default())),
            Arc::new(AlertEngine::default()),
            1,
            4,
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

    /// A probe created *after* the scheduler has started must be picked up by
    /// the reconcile loop and run, without a restart.
    #[tokio::test(start_paused = true)]
    async fn scheduler_schedules_probe_added_after_start() {
        let (_dir, store) = open_store();
        let handle = make_scheduler(Arc::clone(&store), Arc::new(OkProvider)).start();

        // First reconcile (immediate) finds no probes.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        // Add a probe after the scheduler is already running.
        let probe = make_probe(5);
        let probe_id = probe.id.clone();
        store.probes().insert(&probe).unwrap();

        // Advance past the reconcile interval so the new probe is scheduled,
        // then past its run interval so it fires.
        tokio::time::advance(RECONCILE_INTERVAL).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_secs(6 * 60)).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        let runs = store.runs().list_for_probe(&probe_id, 10).unwrap();
        assert!(
            !runs.is_empty(),
            "probe added after start should be scheduled and run"
        );

        handle.shutdown().await;
    }

    /// When a baseline exists for the probe and the provider returns embeddings,
    /// the scheduled run must carry a computed drift report.
    #[tokio::test(start_paused = true)]
    async fn scheduler_attaches_drift_report_when_baseline_exists() {
        use modelsentry_common::models::{BASELINE_SCHEMA_VERSION, BaselineSnapshot};
        use modelsentry_common::types::{BaselineId, RunId};

        let (_dir, store) = open_store();
        let probe = make_probe(5); // single prompt
        let probe_id = probe.id.clone();
        store.probes().insert(&probe).unwrap();

        // One baseline cloud for the single prompt, near the run's embedding.
        let cloud: Vec<Vec<f32>> = [0.04_f32, 0.042, 0.044, 0.046, 0.048, 0.05]
            .iter()
            .map(|&x| vec![x, 0.0, 0.0, 0.0])
            .collect();
        let baseline = BaselineSnapshot {
            id: BaselineId::new(),
            probe_id: probe_id.clone(),
            captured_at: Utc::now(),
            schema_version: BASELINE_SCHEMA_VERSION,
            embedding_model: "test".to_string(),
            prompt_clouds: vec![cloud],
            n_runs: 6,
            run_id: RunId::new(),
        };
        store.baselines().insert(&baseline).unwrap();

        let handle = make_scheduler(Arc::clone(&store), Arc::new(EmbedProvider)).start();
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(6 * 60)).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        let run = store
            .runs()
            .list_for_probe(&probe_id, 1)
            .unwrap()
            .into_iter()
            .next()
            .expect("a run should have been recorded");
        assert!(
            run.drift_report.is_some(),
            "run should carry a drift report when a baseline exists"
        );

        handle.shutdown().await;
    }

    // --- schedule next-run unit tests ---

    #[test]
    fn every_minutes_gives_exact_next_time() {
        let from = Utc::now();
        let s = ProbeSchedule::EveryMinutes { minutes: 15 };
        assert_eq!(
            next_run_after(&s, from),
            from + chrono::Duration::minutes(15)
        );
    }

    #[test]
    fn valid_cron_next_is_within_one_period() {
        // "every minute" — next firing is strictly after `from` and ≤60 s away.
        let from = Utc::now();
        let s = ProbeSchedule::Cron {
            expression: "* * * * *".to_string(),
        };
        let next = next_run_after(&s, from);
        assert!(
            next > from && next <= from + chrono::Duration::seconds(60),
            "expected next within 60 s, got {next}"
        );
    }

    #[test]
    fn invalid_cron_falls_back_to_hourly() {
        let from = Utc::now();
        let s = ProbeSchedule::Cron {
            expression: "not-a-cron-expr".to_string(),
        };
        assert_eq!(next_run_after(&s, from), from + chrono::Duration::hours(1));
    }

    #[test]
    fn zero_minutes_schedule_does_not_busy_loop() {
        // Defensive: a 0-minute probe that slipped past validation is floored to
        // a 1-minute step, never an already-past (busy-looping) next-run time.
        let from = Utc::now();
        let s = ProbeSchedule::EveryMinutes { minutes: 0 };
        assert_eq!(
            next_run_after(&s, from),
            from + chrono::Duration::minutes(1)
        );
    }

    #[test]
    fn parse_cron_schedule_accepts_five_and_six_field() {
        assert!(parse_cron_schedule("0 * * * *").is_ok()); // 5-field
        assert!(parse_cron_schedule("0 0 * * * *").is_ok()); // 6-field
        assert!(parse_cron_schedule("definitely not cron").is_err());
    }

    #[test]
    fn validate_schedule_rejects_bad_inputs_and_accepts_good() {
        assert!(validate_schedule(&ProbeSchedule::EveryMinutes { minutes: 0 }).is_err());
        assert!(validate_schedule(&ProbeSchedule::EveryMinutes { minutes: 1 }).is_ok());
        assert!(
            validate_schedule(&ProbeSchedule::Cron {
                expression: "bogus".to_string()
            })
            .is_err()
        );
        assert!(
            validate_schedule(&ProbeSchedule::Cron {
                expression: "0 * * * *".to_string()
            })
            .is_ok()
        );
    }

    #[test]
    fn six_field_cron_is_parsed_directly() {
        // 6-field: sec min hour dom month dow — "every minute at second 0"
        let from = Utc::now();
        let s = ProbeSchedule::Cron {
            expression: "0 * * * * *".to_string(),
        };
        let next = next_run_after(&s, from);
        assert!(
            next > from && next <= from + chrono::Duration::seconds(60),
            "expected next within 60 s, got {next}"
        );
    }

    /// Restart durability: a probe whose persisted next-run is already in the
    /// past must run immediately on start (a single catch-up), without waiting a
    /// full interval — proven here by advancing **no** virtual time.
    #[tokio::test(start_paused = true)]
    async fn overdue_probe_runs_immediately_via_persisted_schedule() {
        let (_dir, store) = open_store();
        let probe = make_probe(60); // hourly — would otherwise wait an hour
        let probe_id = probe.id.clone();
        store.probes().insert(&probe).unwrap();
        // Simulate a prior run that scheduled the next one for 10 minutes ago
        // (i.e. the daemon was down across it).
        store
            .schedule()
            .set_next_run(&probe_id, Utc::now() - chrono::Duration::minutes(10))
            .unwrap();

        let handle = make_scheduler(Arc::clone(&store), Arc::new(OkProvider)).start();

        // No time advance — only yields. The overdue probe must fire on a
        // zero-length sleep rather than waiting its hourly interval.
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }

        let runs = store.runs().list_for_probe(&probe_id, 10).unwrap();
        assert!(
            !runs.is_empty(),
            "overdue probe should run immediately on start (restart catch-up)"
        );

        handle.shutdown().await;
    }
}
