<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import { api, ApiError } from '$lib/api.js';
  import type { Probe, ProbeRun, BaselineSnapshot, AlertEvent } from '$lib/types.js';
  import DriftMetrics from '$lib/components/DriftMetrics.svelte';
  import DriftChart from '$lib/components/DriftChart.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let probe: Probe | null = $state(null);
  let runs: ProbeRun[] = $state([]);
  let baseline: BaselineSnapshot | null = $state(null);
  let events: AlertEvent[] = $state([]);
  let loading = $state(true);
  let error: string | null = $state(null);

  // Run-now state
  let runningNow = $state(false);
  let runNowError: string | null = $state(null);
  let runNowResult: ProbeRun | null = $state(null);
  let toastVisible = $state(false);

  let probeId = $derived(page.params.id as string);

  let latestRun = $derived(runNowResult ?? runs[0] ?? null);
  let latestReport = $derived(latestRun?.drift_report ?? null);

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function scheduleLabel(p: Probe): string {
    if (p.schedule.kind === 'cron') return p.schedule.expression;
    return `every ${p.schedule.minutes} minutes`;
  }

  function showToast() {
    toastVisible = true;
    setTimeout(() => {
      toastVisible = false;
    }, 4000);
  }

  // ---------------------------------------------------------------------------
  // Run-now
  // ---------------------------------------------------------------------------

  async function handleRunNow() {
    if (!probeId || runningNow) return;
    runningNow = true;
    runNowError = null;
    runNowResult = null;
    try {
      const result = await api.probes.runNow(probeId);
      runNowResult = result;
      // Prepend to runs list for the chart
      runs = [result, ...runs];
      showToast();
    } catch (e) {
      runNowError = e instanceof ApiError ? e.message : String(e);
    } finally {
      runningNow = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Data loading
  // ---------------------------------------------------------------------------

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
      // Filter events to this probe's runs
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

<!-- Toast notification -->
{#if toastVisible}
  <div class="toast" class:visible={toastVisible}>
    ✓ Run complete — drift level: <strong>{runNowResult?.drift_report?.drift_level ?? 'n/a'}</strong>
  </div>
{/if}

<main>
  <header>
    <div class="header-row">
      <div>
        <a class="breadcrumb" href="/probes">← Probes</a>
        <h1>{probe?.name ?? '…'}</h1>
        {#if probe}
          <p class="subtitle">
            {probe.provider.kind.replace('_', ' ')} · {probe.model} ·
            {scheduleLabel(probe)}
          </p>
        {/if}
      </div>

      <button
        class="btn-run"
        onclick={handleRunNow}
        disabled={runningNow || loading}
      >
        {runningNow ? 'Running…' : 'Run Now'}
      </button>
    </div>

    {#if runNowError}
      <p class="error-inline">Run failed: {runNowError}</p>
    {/if}
  </header>

  {#if loading}
    <p class="status-msg">Loading…</p>
  {:else if error}
    <p class="error-msg">Failed to load probe: {error}</p>
  {:else if probe}
    <div class="layout">
      <!-- Left column: chart + run table -->
      <div class="main-col">
        <!-- Drift chart -->
        <section class="section">
          <h2>Drift History</h2>
          <DriftChart
            probeId={probe.name}
            {runs}
            {baseline}
          />
        </section>

        <!-- Prompts -->
        <section class="section">
          <h2>Prompts ({probe.prompts.length})</h2>
          <ul class="prompt-list">
            {#each probe.prompts as prompt (prompt.id)}
              <li class="prompt-item">
                <p class="prompt-text">{prompt.text}</p>
                {#if prompt.expected_contains}
                  <span class="constraint constraint-ok">must contain: <em>{prompt.expected_contains}</em></span>
                {/if}
                {#if prompt.expected_not_contains}
                  <span class="constraint constraint-err">must not contain: <em>{prompt.expected_not_contains}</em></span>
                {/if}
              </li>
            {/each}
          </ul>
        </section>

        <!-- Recent runs table -->
        <section class="section">
          <h2>Recent Runs</h2>
          {#if runs.length === 0}
            <p class="empty">No runs yet.</p>
          {:else}
            <div class="table-wrap">
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
                      <td class="meta">{new Date(run.started_at).toLocaleString()}</td>
                      <td>
                        <span class="run-status" data-status={run.status}>
                          {run.status.replace('_', ' ')}
                        </span>
                      </td>
                      <td class="num">
                        {run.drift_report?.kl_divergence.toFixed(4) ?? '—'}
                      </td>
                      <td class="num">
                        {run.drift_report?.cosine_distance.toFixed(4) ?? '—'}
                      </td>
                      <td>
                        {#if run.drift_report}
                          <span class="badge" data-level={run.drift_report.drift_level}>
                            {run.drift_report.drift_level}
                          </span>
                        {:else}
                          <span class="na">—</span>
                        {/if}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        </section>
      </div>

      <!-- Right sidebar: latest metrics + alerts -->
      <aside class="sidebar">
        <!-- Latest drift metrics -->
        <section class="section">
          <h2>Latest Metrics</h2>
          {#if latestReport}
            <DriftMetrics report={latestReport} />
          {:else}
            <p class="empty">No drift report yet.</p>
          {/if}
        </section>

        <!-- Last 5 alert events for this probe -->
        <section class="section">
          <h2>Alert Events</h2>
          {#if events.length === 0}
            <p class="empty">No alerts for this probe.</p>
          {:else}
            <ul class="event-list">
              {#each events.slice(0, 5) as event (event.id)}
                <li class="event-item" class:acked={event.acknowledged}>
                  <div class="event-row">
                    <span class="badge" data-level={event.drift_report.drift_level}>
                      {event.drift_report.drift_level}
                    </span>
                    {#if event.acknowledged}
                      <span class="ack-badge">ack'd</span>
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
        </section>

        <!-- Baseline info -->
        {#if baseline}
          <section class="section">
            <h2>Baseline</h2>
            <p class="baseline-meta">
              Captured {new Date(baseline.captured_at).toLocaleString()}
            </p>
            <p class="baseline-meta">
              Variance {baseline.embedding_variance.toFixed(6)}
            </p>
          </section>
        {/if}
      </aside>
    </div>
  {/if}
</main>

<style>
  /* ---- Toast ---- */
  .toast {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    background: #0f172a;
    color: #fff;
    border-radius: 0.5rem;
    padding: 0.75rem 1.25rem;
    font-size: 0.875rem;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
    opacity: 0;
    transform: translateY(8px);
    transition: opacity 0.25s, transform 0.25s;
    z-index: 100;
  }

  .toast.visible {
    opacity: 1;
    transform: translateY(0);
  }

  /* ---- Header ---- */
  header {
    margin-bottom: 1.75rem;
  }

  .header-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
  }

  .breadcrumb {
    font-size: 0.8rem;
    color: #64748b;
    text-decoration: none;
    display: block;
    margin-bottom: 0.25rem;
  }

  .breadcrumb:hover {
    color: #2563eb;
  }

  h1 {
    margin: 0;
    font-size: 1.5rem;
    font-weight: 800;
    color: #0f172a;
  }

  h2 {
    margin: 0 0 0.75rem;
    font-size: 0.95rem;
    font-weight: 700;
    color: #1e293b;
  }

  .subtitle {
    margin: 0.2rem 0 0;
    color: #64748b;
    font-size: 0.875rem;
  }

  .btn-run {
    margin-top: 0.25rem;
    padding: 0.5rem 1.25rem;
    background: #2563eb;
    color: #fff;
    border: none;
    border-radius: 0.4rem;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }

  .btn-run:hover:not(:disabled) {
    background: #1d4ed8;
  }

  .btn-run:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .error-inline {
    color: #dc2626;
    font-size: 0.85rem;
    margin: 0.5rem 0 0;
  }

  /* ---- Layout ---- */
  .layout {
    display: grid;
    grid-template-columns: 1fr 280px;
    gap: 1.5rem;
    align-items: start;
  }

  @media (max-width: 700px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }

  .section {
    background: #fff;
    border: 1px solid #e2e8f0;
    border-radius: 0.75rem;
    padding: 1rem 1.25rem;
    margin-bottom: 1.25rem;
  }

  /* ---- Prompts ---- */
  .prompt-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .prompt-item {
    background: #f8fafc;
    border: 1px solid #e2e8f0;
    border-radius: 0.5rem;
    padding: 0.75rem 1rem;
  }

  .prompt-text {
    margin: 0 0 0.4rem;
    font-size: 0.875rem;
    color: #0f172a;
    line-height: 1.5;
  }

  .constraint {
    display: inline-block;
    font-size: 0.75rem;
    padding: 0.1rem 0.5rem;
    border-radius: 0.25rem;
    margin-right: 0.4rem;
  }

  .constraint-ok  { background: #dcfce7; color: #15803d; }
  .constraint-err { background: #fee2e2; color: #dc2626; }

  /* ---- Runs table ---- */
  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }

  thead tr {
    border-bottom: 2px solid #e2e8f0;
  }

  th {
    text-align: left;
    padding: 0.4rem 0.6rem;
    font-weight: 600;
    color: #475569;
    white-space: nowrap;
  }

  tbody tr {
    border-bottom: 1px solid #f1f5f9;
  }

  tbody tr.highlighted {
    background: #eff6ff;
  }

  td {
    padding: 0.4rem 0.6rem;
    vertical-align: middle;
  }

  .meta { color: #64748b; }
  .num  { font-variant-numeric: tabular-nums; color: #334155; }
  .na   { color: #94a3b8; }
  .empty { color: #64748b; font-size: 0.875rem; }

  .run-status {
    font-size: 0.7rem;
    font-weight: 700;
    text-transform: uppercase;
  }

  .run-status[data-status='success']         { color: #16a34a; }
  .run-status[data-status='partial_failure'] { color: #d97706; }
  .run-status[data-status='failed']          { color: #dc2626; }

  /* ---- Drift level badge ---- */
  .badge {
    display: inline-block;
    padding: 0.15rem 0.55rem;
    border-radius: 999px;
    font-size: 0.7rem;
    font-weight: 700;
    text-transform: uppercase;
    background: #e2e8f0;
    color: #334155;
  }

  .badge[data-level='low']      { background: #dbeafe; color: #2563eb; }
  .badge[data-level='medium']   { background: #fef3c7; color: #d97706; }
  .badge[data-level='high']     { background: #fee2e2; color: #dc2626; }
  .badge[data-level='critical'] { background: #dc2626; color: #fff; }

  /* ---- Sidebar event list ---- */
  .event-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .event-item {
    padding: 0.5rem 0.75rem;
    background: #f8fafc;
    border-radius: 0.5rem;
    border: 1px solid #e2e8f0;
  }

  .event-item.acked {
    opacity: 0.55;
  }

  .event-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
  }

  .ack-badge {
    font-size: 0.65rem;
    background: #dcfce7;
    color: #16a34a;
    border-radius: 999px;
    padding: 0.1rem 0.45rem;
    font-weight: 600;
  }

  .event-meta {
    margin: 0;
    font-size: 0.72rem;
    color: #64748b;
  }

  /* ---- Baseline section ---- */
  .baseline-meta {
    margin: 0.2rem 0;
    font-size: 0.8rem;
    color: #64748b;
  }

  /* ---- General ---- */
  .status-msg,
  .error-msg {
    text-align: center;
    padding: 3rem 0;
    color: #64748b;
  }

  .error-msg { color: #dc2626; }
</style>
