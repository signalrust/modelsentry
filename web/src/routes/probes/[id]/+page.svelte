<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import { api, ApiError } from '$lib/api.js';
  import type { Probe, ProbeRun, BaselineSnapshot, AlertEvent } from '$lib/types.js';
  import DriftMetrics from '$lib/components/DriftMetrics.svelte';
  import DriftChart from '$lib/components/DriftChart.svelte';

  let probe: Probe | null = $state(null);
  let runs: ProbeRun[] = $state([]);
  let baseline: BaselineSnapshot | null = $state(null);
  let events: AlertEvent[] = $state([]);
  let loading = $state(true);
  let error: string | null = $state(null);

  let runningNow = $state(false);
  let runNowError: string | null = $state(null);
  let runNowResult: ProbeRun | null = $state(null);
  let toastVisible = $state(false);

  let probeId = $derived(page.params.id as string);
  let latestRun = $derived(runNowResult ?? runs[0] ?? null);
  let latestReport = $derived(latestRun?.drift_report ?? null);

  function scheduleLabel(p: Probe): string {
    if (p.schedule.kind === 'cron') return p.schedule.expression;
    return `every ${p.schedule.minutes} min`;
  }

  function showToast() {
    toastVisible = true;
    setTimeout(() => { toastVisible = false; }, 4000);
  }

  async function handleRunNow() {
    if (!probeId || runningNow) return;
    runningNow = true;
    runNowError = null;
    runNowResult = null;
    try {
      const result = await api.probes.runNow(probeId);
      runNowResult = result;
      runs = [result, ...runs];
      showToast();
    } catch (e) {
      runNowError = e instanceof ApiError ? e.message : String(e);
    } finally {
      runningNow = false;
    }
  }

  onMount(async () => {
    try {
      const id = probeId;
      const [fetchedProbe, fetchedRuns, fetchedEvents] = await Promise.all([
        api.probes.get(id),
        api.runs.listForProbe(id, 30),
        api.alerts.listEvents(5),
      ]);
      probe = fetchedProbe;
      runs = fetchedRuns;
      const runIdSet = new Set(fetchedRuns.map((r) => r.id));
      events = fetchedEvents.filter((e) => runIdSet.has(e.drift_report.run_id));
      baseline = await api.baselines.getLatestForProbe(id).catch(() => null);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  });
</script>

<svelte:head>
  <title>ModelSentry — {probe?.name ?? 'Probe'}</title>
</svelte:head>

<!-- Toast -->
{#if toastVisible}
  <div class="toast visible">
    ✓ Run complete — drift: <strong>{runNowResult?.drift_report?.drift_level ?? 'n/a'}</strong>
  </div>
{/if}

<!-- Page header -->
<div class="page-header">
  <div>
    <a class="breadcrumb" href="/probes">← Probes</a>
    <h1 class="page-title">{probe?.name ?? '…'}</h1>
    {#if probe}
      <p class="page-subtitle">
        {probe.provider.kind.replace('_', ' ')} · {probe.model} · {scheduleLabel(probe)}
      </p>
    {/if}
  </div>
  <div class="page-actions">
    <button
      class="btn btn-primary"
      onclick={handleRunNow}
      disabled={runningNow || loading}
    >
      {runningNow ? '⟳ Running…' : '▶ Run Now'}
    </button>
  </div>
</div>

{#if runNowError}
  <div class="error-banner" style="margin-bottom: var(--sp-4)">Run failed: {runNowError}</div>
{/if}

{#if loading}
  <p class="loading-state">Loading…</p>
{:else if error}
  <div class="error-banner">Failed to load probe: {error}</div>
{:else if probe}
  <div class="two-col-layout">

    <!-- ── Left: chart + prompts + runs ── -->
    <div class="main-col">

      <div class="section">
        <h2>Drift History</h2>
        <DriftChart probeId={probe.name} {runs} {baseline} />
      </div>

      <div class="section">
        <h2>Prompts ({probe.prompts.length})</h2>
        <ul class="prompt-list">
          {#each probe.prompts as prompt (prompt.id)}
            <li class="prompt-item">
              <p class="prompt-text">{prompt.text}</p>
              {#if prompt.expected_contains}
                <span class="constraint constraint-ok">✓ must contain: <em>{prompt.expected_contains}</em></span>
              {/if}
              {#if prompt.expected_not_contains}
                <span class="constraint constraint-err">✗ must not contain: <em>{prompt.expected_not_contains}</em></span>
              {/if}
            </li>
          {/each}
        </ul>
      </div>

      <div class="section">
        <h2>Recent Runs</h2>
        {#if runs.length === 0}
          <p class="empty-state" style="padding: var(--sp-6) 0">No runs yet.</p>
        {:else}
          <div class="table-container">
            <table>
              <thead>
                <tr>
                  <th>Started</th>
                  <th>Status</th>
                  <th>KL Div.</th>
                  <th>Cosine</th>
                  <th>Drift Level</th>
                </tr>
              </thead>
              <tbody>
                {#each runs.slice(0, 15) as run (run.id)}
                  <tr class:highlighted={run.id === runNowResult?.id}>
                    <td class="td-meta">{new Date(run.started_at).toLocaleString()}</td>
                    <td>
                      <span class="run-status" data-status={run.status}>
                        {run.status.replace('_', ' ')}
                      </span>
                    </td>
                    <td class="td-num">{run.drift_report?.kl_divergence.toFixed(4) ?? '—'}</td>
                    <td class="td-num">{run.drift_report?.cosine_distance.toFixed(4) ?? '—'}</td>
                    <td>
                      {#if run.drift_report}
                        <span class="badge" data-level={run.drift_report.drift_level}>
                          {run.drift_report.drift_level}
                        </span>
                      {:else}
                        <span class="td-meta">—</span>
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>

    </div>

    <!-- ── Right: metrics + alerts + baseline ── -->
    <aside class="detail-aside">

      <div class="section">
        <h2>Latest Metrics</h2>
        {#if latestReport}
          <DriftMetrics report={latestReport} />
        {:else}
          <p class="empty-state" style="padding: var(--sp-4) 0">No drift report yet.</p>
        {/if}
      </div>

      <div class="section">
        <h2>Alert Events</h2>
        {#if events.length === 0}
          <p class="empty-state" style="padding: var(--sp-4) 0">No alerts for this probe.</p>
        {:else}
          <ul class="event-list">
            {#each events.slice(0, 5) as event (event.id)}
              <li class="event-item" class:acked={event.acknowledged}>
                <div class="event-row">
                  <span class="badge" data-level={event.drift_report.drift_level}>
                    {event.drift_report.drift_level}
                  </span>
                  {#if event.acknowledged}
                    <span class="badge badge-success">ack'd</span>
                  {/if}
                </div>
                <p class="event-meta">
                  KL {event.drift_report.kl_divergence.toFixed(3)} ·
                  {new Date(event.fired_at).toLocaleString()}
                </p>
              </li>
            {/each}
          </ul>
        {/if}
      </div>

      {#if baseline}
        <div class="section">
          <h2>Baseline</h2>
          <p class="baseline-meta">Captured {new Date(baseline.captured_at).toLocaleString()}</p>
          <p class="baseline-meta">Variance {baseline.embedding_variance.toFixed(6)}</p>
        </div>
      {/if}

    </aside>

  </div>
{/if}

<style>
  /* Two-column detail layout — overrides global when in aside context */
  .main-col { min-width: 0; }
  .detail-aside { min-width: 0; }

  /* Prompt list */
  .prompt-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .prompt-item {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }
  .prompt-text {
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.5;
  }
  .constraint {
    font-size: var(--text-xs);
    font-family: var(--font-mono);
    padding: 2px 8px;
    border-radius: var(--r-sm);
    display: inline-block;
  }
  .constraint-ok  { background: rgba(34,197,94,0.1);  color: var(--semantic-up);   }
  .constraint-err { background: rgba(239,68,68,0.1);  color: var(--semantic-down); }

  /* Highlighted new run row */
  tr.highlighted td { background: rgba(59, 130, 246, 0.08); }

  /* Event list */
  .event-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .event-item {
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    background: var(--bg-input);
  }
  .event-item.acked { opacity: 0.5; }
  .event-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-bottom: 4px;
  }
  .event-meta {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-family: var(--font-mono);
  }

  /* Baseline */
  .baseline-meta {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-family: var(--font-mono);
    margin-bottom: var(--sp-1);
  }
</style>
