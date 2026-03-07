<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { Probe, ProbeRun, BaselineSnapshot, AlertEvent } from '$lib/types.js';
  import SummaryCard from '$lib/components/SummaryCard.svelte';
  import DriftChart from '$lib/components/DriftChart.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let probes: Probe[] = $state([]);
  let runsMap: Record<string, ProbeRun[]> = $state({});
  let baselineMap: Record<string, BaselineSnapshot | null> = $state({});
  let events: AlertEvent[] = $state([]);
  let loading = $state(true);
  let error: string | null = $state(null);

  // ---------------------------------------------------------------------------
  // Derived summary values
  // ---------------------------------------------------------------------------

  let totalProbes = $derived(probes.length);

  let activeAlerts = $derived(events.filter((e) => !e.acknowledged).length);

  let lastRunStatus = $derived.by(() => {
    const allRuns = Object.values(runsMap).flat();
    if (allRuns.length === 0) return 'neutral' as const;
    const latest = allRuns.reduce((a, b) =>
      new Date(a.started_at) > new Date(b.started_at) ? a : b,
    );
    if (latest.status === 'success') return 'ok' as const;
    if (latest.status === 'partial_failure') return 'warn' as const;
    return 'error' as const;
  });

  let lastRunLabel = $derived.by(() => {
    const allRuns = Object.values(runsMap).flat();
    if (allRuns.length === 0) return '—';
    const latest = allRuns.reduce((a, b) =>
      new Date(a.started_at) > new Date(b.started_at) ? a : b,
    );
    return latest.status.replace('_', ' ');
  });

  let highDriftProbes = $derived(probes.filter((p) => {
    const runs = runsMap[p.id] ?? [];
    const level = runs[0]?.drift_report?.drift_level;
    return level === 'high' || level === 'critical';
  }).length);

  let alertCardStatus = $derived.by(() => {
    if (activeAlerts === 0) return 'ok' as const;
    if (highDriftProbes > 0) return 'error' as const;
    return 'warn' as const;
  });

  // ---------------------------------------------------------------------------
  // Data loading — all fetches in parallel
  // ---------------------------------------------------------------------------

  onMount(async () => {
    try {
      const [fetchedProbes, fetchedEvents] = await Promise.all([
        api.probes.list(),
        api.alerts.listEvents(20),
      ]);
      probes = fetchedProbes;
      events = fetchedEvents;

      const perProbe = await Promise.all(
        probes.map(async (p) => {
          const [runs, baseline] = await Promise.all([
            api.runs.listForProbe(p.id, 20).catch(() => [] as ProbeRun[]),
            api.baselines
              .getLatestForProbe(p.id)
              .catch(() => null as BaselineSnapshot | null),
          ]);
          return { id: p.id, runs, baseline };
        }),
      );

      runsMap = Object.fromEntries(perProbe.map((r) => [r.id, r.runs]));
      baselineMap = Object.fromEntries(perProbe.map((r) => [r.id, r.baseline]));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  });
</script>

<svelte:head>
  <title>ModelSentry — Dashboard</title>
</svelte:head>

<div class="page-header">
  <div>
    <h1 class="page-title">Dashboard</h1>
    <p class="page-subtitle">LLM drift detection at a glance</p>
  </div>
  <div class="page-actions">
    <a class="btn btn-primary btn-sm" href="/probes">Manage Probes →</a>
  </div>
</div>

{#if loading}
  <p class="loading-state">Loading…</p>
{:else if error}
  <div class="error-banner">Failed to load dashboard: {error}</div>
{:else}
  <!-- KPI row -->
  <div class="grid grid-4 section">
    <SummaryCard title="Probes" value={totalProbes} status="neutral" />
    <SummaryCard title="Last Run" value={lastRunLabel} status={lastRunStatus} />
    <SummaryCard title="Active Alerts" value={activeAlerts} status={alertCardStatus} />
    <SummaryCard title="High Drift" value={highDriftProbes} status={highDriftProbes > 0 ? 'error' : 'ok'} />
  </div>

  <!-- Drift charts -->
  {#if probes.length === 0}
    <div class="empty-state">
      No probes configured yet.
      <a href="/probes" class="btn btn-primary btn-sm" style="margin-left:var(--sp-3)">Add a probe →</a>
    </div>
  {:else}
    <div class="section">
      <div class="section">
        <h2>Drift Charts</h2>
      </div>
      <div class="charts-grid">
        {#each probes as probe (probe.id)}
          <a class="chart-link" href="/probes/{probe.id}">
            <DriftChart
              probeId={probe.name}
              runs={runsMap[probe.id] ?? []}
              baseline={baselineMap[probe.id] ?? null}
            />
          </a>
        {/each}
      </div>
    </div>
  {/if}

  <!-- Recent alert events -->
  {#if events.length > 0}
    <div class="section">
      <h2>Recent Alert Events</h2>
      <div class="card" style="padding: 0; overflow: hidden;">
        <div class="table-container">
          <table>
            <thead>
              <tr>
                <th>Time</th>
                <th>Drift Level</th>
                <th>KL Divergence</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {#each events.slice(0, 10) as event (event.id)}
                <tr class:acked={event.acknowledged}>
                  <td class="td-meta">{new Date(event.fired_at).toLocaleString()}</td>
                  <td>
                    <span class="badge" data-level={event.drift_report.drift_level}>
                      {event.drift_report.drift_level}
                    </span>
                  </td>
                  <td class="td-num">{event.drift_report.kl_divergence.toFixed(3)}</td>
                  <td>
                    {#if event.acknowledged}
                      <span class="badge badge-success">ack'd</span>
                    {:else}
                      <span class="badge badge-warning">pending</span>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  {/if}
{/if}

<style>
  .charts-grid {
    display: grid;
    gap: var(--sp-4);
    grid-template-columns: 1fr;
  }
  @media (min-width: 900px) {
    .charts-grid { grid-template-columns: repeat(2, 1fr); }
  }

  .chart-link {
    display: block;
    text-decoration: none;
    transition: transform var(--transition), box-shadow var(--transition);
    border-radius: var(--r-md);
  }
  .chart-link:hover {
    transform: translateY(-2px);
    box-shadow: var(--shadow-glow);
  }

  tr.acked { opacity: 0.5; }
</style>
