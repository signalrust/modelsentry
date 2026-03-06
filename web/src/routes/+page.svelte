<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { Probe, ProbeRun, BaselineSnapshot, AlertEvent } from '$lib/types.js';
  import SummaryCard from '$lib/components/SummaryCard.svelte';
  import DriftChart from '$lib/components/DriftChart.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let probes: Probe[] = [];
  let runsMap: Record<string, ProbeRun[]> = {};
  let baselineMap: Record<string, BaselineSnapshot | null> = {};
  let events: AlertEvent[] = [];
  let loading = true;
  let error: string | null = null;

  // ---------------------------------------------------------------------------
  // Derived summary values
  // ---------------------------------------------------------------------------

  $: totalProbes = probes.length;

  $: activeAlerts = events.filter((e) => !e.acknowledged).length;

  $: lastRunStatus = (() => {
    const allRuns = Object.values(runsMap).flat();
    if (allRuns.length === 0) return 'neutral' as const;
    const latest = allRuns.reduce((a, b) =>
      new Date(a.started_at) > new Date(b.started_at) ? a : b,
    );
    if (latest.status === 'success') return 'ok' as const;
    if (latest.status === 'partial_failure') return 'warn' as const;
    return 'error' as const;
  })();

  $: lastRunLabel = (() => {
    const allRuns = Object.values(runsMap).flat();
    if (allRuns.length === 0) return '—';
    const latest = allRuns.reduce((a, b) =>
      new Date(a.started_at) > new Date(b.started_at) ? a : b,
    );
    return latest.status.replace('_', ' ');
  })();

  $: highDriftProbes = probes.filter((p) => {
    const runs = runsMap[p.id] ?? [];
    const level = runs[0]?.drift_report?.drift_level;
    return level === 'high' || level === 'critical';
  }).length;

  $: alertCardStatus = (() => {
    if (activeAlerts === 0) return 'ok' as const;
    if (highDriftProbes > 0) return 'error' as const;
    return 'warn' as const;
  })();

  // ---------------------------------------------------------------------------
  // Data loading — all fetches in parallel
  // ---------------------------------------------------------------------------

  onMount(async () => {
    try {
      // Load probes + events in parallel first.
      const [fetchedProbes, fetchedEvents] = await Promise.all([
        api.probes.list(),
        api.alerts.listEvents(20),
      ]);
      probes = fetchedProbes;
      events = fetchedEvents;

      // Then load each probe's last 7 runs and latest baseline in parallel.
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

<main>
  <header>
    <h1>ModelSentry</h1>
    <p class="subtitle">LLM Drift Detection Dashboard</p>
  </header>

  {#if loading}
    <p class="status-msg">Loading…</p>
  {:else if error}
    <p class="error-msg">Failed to load dashboard: {error}</p>
  {:else}
    <!-- Summary cards -->
    <section class="cards">
      <SummaryCard title="Probes" value={totalProbes} status="neutral" />
      <SummaryCard title="Last Run" value={lastRunLabel} status={lastRunStatus} />
      <SummaryCard
        title="Active Alerts"
        value={activeAlerts}
        status={alertCardStatus}
      />
      <SummaryCard
        title="High Drift"
        value={highDriftProbes}
        status={highDriftProbes > 0 ? 'error' : 'ok'}
      />
    </section>

    <!-- Drift charts — one per probe -->
    {#if probes.length === 0}
      <p class="empty">No probes configured yet. <a href="/probes">Add one →</a></p>
    {:else}
      <section class="charts">
        {#each probes as probe (probe.id)}
          <DriftChart
            probeId={probe.name}
            runs={runsMap[probe.id] ?? []}
            baseline={baselineMap[probe.id] ?? null}
          />
        {/each}
      </section>
    {/if}

    <!-- Recent alert events -->
    {#if events.length > 0}
      <section class="events">
        <h2>Recent Alert Events</h2>
        <ul>
          {#each events.slice(0, 10) as event (event.id)}
            <li class:acked={event.acknowledged}>
              <span class="event-time">{new Date(event.fired_at).toLocaleString()}</span>
              <span class="event-level" data-level={event.drift_report.drift_level}>
                {event.drift_report.drift_level}
              </span>
              <span class="event-kl">KL {event.drift_report.kl_divergence.toFixed(3)}</span>
              {#if event.acknowledged}
                <span class="ack-badge">ack'd</span>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}
  {/if}
</main>

<style>
  header {
    margin-bottom: 2rem;
  }

  h1 {
    margin: 0;
    font-size: 1.75rem;
    font-weight: 800;
    color: #0f172a;
  }

  .subtitle {
    margin: 0.25rem 0 0;
    color: #64748b;
    font-size: 0.9rem;
  }

  .status-msg,
  .error-msg,
  .empty {
    text-align: center;
    padding: 3rem 0;
    color: #64748b;
  }

  .error-msg {
    color: #dc2626;
  }

  /* Summary cards row */
  .cards {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
    margin-bottom: 2rem;
  }

  /* Charts grid */
  .charts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(420px, 1fr));
    gap: 1.25rem;
    margin-bottom: 2rem;
  }

  /* Events list */
  .events h2 {
    font-size: 1rem;
    font-weight: 600;
    margin: 0 0 0.75rem;
    color: #0f172a;
  }

  .events ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .events li {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    background: #fff;
    border: 1px solid #e2e8f0;
    border-radius: 0.5rem;
    padding: 0.6rem 1rem;
    font-size: 0.875rem;
  }

  .events li.acked {
    opacity: 0.55;
  }

  .event-time {
    color: #64748b;
    flex-shrink: 0;
  }

  .event-level {
    font-weight: 700;
    text-transform: uppercase;
    font-size: 0.75rem;
    padding: 0.15rem 0.5rem;
    border-radius: 999px;
    background: #e2e8f0;
    color: #334155;
  }

  .event-level[data-level='high'],
  .event-level[data-level='critical'] {
    background: #fee2e2;
    color: #dc2626;
  }

  .event-level[data-level='medium'] {
    background: #fef3c7;
    color: #d97706;
  }

  .event-level[data-level='low'] {
    background: #dbeafe;
    color: #2563eb;
  }

  .event-kl {
    color: #475569;
  }

  .ack-badge {
    margin-left: auto;
    font-size: 0.7rem;
    background: #dcfce7;
    color: #16a34a;
    border-radius: 999px;
    padding: 0.1rem 0.5rem;
    font-weight: 600;
  }
</style>

