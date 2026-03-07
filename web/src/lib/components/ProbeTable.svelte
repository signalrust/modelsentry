<script lang="ts">
  import type { Probe, ProbeRun, DriftLevel } from '$lib/types.js';

  let { probes, latestRunMap = {} }: {
    probes: Probe[];
    latestRunMap?: Record<string, ProbeRun | null>;
  } = $props();

  // ---------------------------------------------------------------------------
  // Sorting
  // ---------------------------------------------------------------------------

  type SortKey = 'name' | 'drift';
  let sortKey: SortKey = $state('name');
  let sortAsc = $state(true);

  const DRIFT_ORDER: Record<DriftLevel, number> = {
    none: 0,
    low: 1,
    medium: 2,
    high: 3,
    critical: 4,
  };

  function driftOf(probeId: string): DriftLevel {
    return latestRunMap[probeId]?.drift_report?.drift_level ?? 'none';
  }

  function toggleSort(key: SortKey) {
    if (sortKey === key) {
      sortAsc = !sortAsc;
    } else {
      sortKey = key;
      sortAsc = true;
    }
  }

  let sorted = $derived([...probes].sort((a, b) => {
    let cmp = 0;
    if (sortKey === 'name') {
      cmp = a.name.localeCompare(b.name);
    } else {
      cmp = DRIFT_ORDER[driftOf(a.id)] - DRIFT_ORDER[driftOf(b.id)];
    }
    return sortAsc ? cmp : -cmp;
  }));

  function scheduleLabel(probe: Probe): string {
    if (probe.schedule.kind === 'cron') return probe.schedule.expression;
    return `every ${probe.schedule.minutes}m`;
  }

  function providerLabel(probe: Probe): string {
    return probe.provider.kind.replace('_', ' ');
  }
</script>

{#if probes.length === 0}
  <p class="empty">No probes configured yet.</p>
{:else}
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th>
            <button class="sort-btn" onclick={() => toggleSort('name')}>
              Name {sortKey === 'name' ? (sortAsc ? '▲' : '▼') : ''}
            </button>
          </th>
          <th>Provider / Model</th>
          <th>Schedule</th>
          <th>
            <button class="sort-btn" onclick={() => toggleSort('drift')}>
              Last Drift {sortKey === 'drift' ? (sortAsc ? '▲' : '▼') : ''}
            </button>
          </th>
          <th>Last Run</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each sorted as probe (probe.id)}
          {@const run = latestRunMap[probe.id] ?? null}
          {@const level = driftOf(probe.id)}
          <tr>
            <td class="probe-name">
              <a href="/probes/{probe.id}">{probe.name}</a>
            </td>
            <td class="meta">{providerLabel(probe)} / {probe.model}</td>
            <td class="meta">{scheduleLabel(probe)}</td>
            <td>
              <span class="badge" data-level={level}>{level}</span>
            </td>
            <td class="meta">
              {#if run}
                {new Date(run.started_at).toLocaleString()}
                <span class="run-status" data-status={run.status}>{run.status.replace('_', ' ')}</span>
              {:else}
                <span class="never">never</span>
              {/if}
            </td>
            <td>
              <a class="btn-link" href="/probes/{probe.id}">Details →</a>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .empty {
    color: #64748b;
    text-align: center;
    padding: 2rem 0;
  }

  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.875rem;
  }

  thead tr {
    border-bottom: 2px solid #e2e8f0;
  }

  th {
    text-align: left;
    padding: 0.6rem 0.75rem;
    font-weight: 600;
    color: #475569;
    white-space: nowrap;
  }

  .sort-btn {
    background: none;
    border: none;
    cursor: pointer;
    font: inherit;
    font-weight: 600;
    color: #475569;
    padding: 0;
  }

  .sort-btn:hover {
    color: #0f172a;
  }

  tbody tr {
    border-bottom: 1px solid #f1f5f9;
    transition: background 0.1s;
  }

  tbody tr:hover {
    background: #f8fafc;
  }

  td {
    padding: 0.6rem 0.75rem;
    vertical-align: middle;
  }

  .probe-name a {
    font-weight: 600;
    color: #2563eb;
    text-decoration: none;
  }

  .probe-name a:hover {
    text-decoration: underline;
  }

  .meta {
    color: #64748b;
  }

  /* Drift level badge */
  .badge {
    display: inline-block;
    padding: 0.15rem 0.6rem;
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

  /* Run status inline chip */
  .run-status {
    margin-left: 0.4rem;
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    opacity: 0.75;
  }

  .run-status[data-status='success']         { color: #16a34a; }
  .run-status[data-status='partial_failure'] { color: #d97706; }
  .run-status[data-status='failed']          { color: #dc2626; }

  .never {
    color: #94a3b8;
    font-style: italic;
  }

  .btn-link {
    font-size: 0.8rem;
    color: #2563eb;
    text-decoration: none;
    white-space: nowrap;
  }

  .btn-link:hover {
    text-decoration: underline;
  }
</style>
