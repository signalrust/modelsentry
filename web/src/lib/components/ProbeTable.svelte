<script lang="ts">
  import { providerModel, type Probe, type ProbeRun, type DriftLevel } from '$lib/types.js';
  import { api, ApiError } from '$lib/api.js';
  import { DRIFT_ORDER, PROVIDER_LABELS, type ProviderKindTag } from '$lib/constants.js';

  let { probes, latestRunMap = {}, onRunUpdated }: {
    probes: Probe[];
    latestRunMap?: Record<string, ProbeRun | null>;
    onRunUpdated?: (probeId: string, run: ProbeRun) => void;
  } = $props();

  // ---------------------------------------------------------------------------
  // Sorting
  // ---------------------------------------------------------------------------

  type SortKey = 'name' | 'drift';
  let sortKey: SortKey = $state('name');
  let sortAsc = $state(true);
  let runningId: string | null = $state(null);
  let runError: string | null = $state(null);

  function driftOf(probeId: string): DriftLevel {
    return latestRunMap[probeId]?.drift_report?.drift_level ?? 'none';
  }

  function toggleSort(key: SortKey) {
    if (sortKey === key) { sortAsc = !sortAsc; } else { sortKey = key; sortAsc = true; }
  }

  let sorted = $derived([...probes].sort((a, b) => {
    let cmp = sortKey === 'name'
      ? a.name.localeCompare(b.name)
      : DRIFT_ORDER[driftOf(a.id)] - DRIFT_ORDER[driftOf(b.id)];
    return sortAsc ? cmp : -cmp;
  }));

  function scheduleLabel(probe: Probe): string {
    if (probe.schedule.kind === 'cron') return probe.schedule.expression;
    return `every ${probe.schedule.minutes}m`;
  }

  function providerLabel(probe: Probe): string {
    return PROVIDER_LABELS[probe.provider.kind as ProviderKindTag] ?? probe.provider.kind.replaceAll('_', ' ');
  }

  async function runNow(probe: Probe) {
    if (runningId) return;
    runningId = probe.id;
    runError = null;
    try {
      const result = await api.probes.runNow(probe.id);
      onRunUpdated?.(probe.id, result);
    } catch (e) {
      runError = `Run failed for "${probe.name}": ${e instanceof ApiError ? e.message : String(e)}`;
    } finally {
      runningId = null;
    }
  }
</script>

{#if runError}
  <div class="error-banner table-run-error" role="alert">{runError}</div>
{/if}

{#if probes.length === 0}
  <p class="empty-state">No probes configured yet.</p>
{:else}
  <div class="table-container">
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
            <td class="td-name">
              <a href="/probes/{probe.id}">{probe.name}</a>
            </td>
            <td class="td-meta">{providerLabel(probe)} / {providerModel(probe.provider)}</td>
            <td class="td-meta">{scheduleLabel(probe)}</td>
            <td>
              <span class="badge" data-level={level}>{level}</span>
            </td>
            <td class="td-meta">
              {#if run}
                {new Date(run.started_at).toLocaleString()}
                <span class="run-status" data-status={run.status}>&nbsp;{run.status.replace('_', ' ')}</span>
              {:else}
                <span class="td-never">never</span>
              {/if}
            </td>
            <td class="actions-cell">
              <button
                class="btn btn-sm"
                onclick={() => runNow(probe)}
                disabled={runningId === probe.id}
                title="Run probe now"
              >
                {runningId === probe.id ? '⟳' : '▶'}
              </button>
              <a class="btn btn-sm" href="/probes/{probe.id}">Details</a>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .actions-cell {
    display: flex;
    gap: var(--sp-1);
    align-items: center;
    white-space: nowrap;
  }
  .td-never {
    color: var(--text-muted);
    font-style: italic;
  }
  .table-run-error {
    margin-bottom: var(--sp-3);
  }
</style>
