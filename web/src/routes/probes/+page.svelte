<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { Probe, ProbeRun } from '$lib/types.js';
  import ProbeTable from '$lib/components/ProbeTable.svelte';
  import AddProbeForm from '$lib/components/AddProbeForm.svelte';

  let probes: Probe[] = $state([]);
  let latestRunMap: Record<string, ProbeRun | null> = $state({});
  let loading = $state(true);
  let error: string | null = $state(null);
  let showForm = $state(false);

  onMount(async () => {
    try {
      probes = await api.probes.list();
      const results = await Promise.all(
        probes.map(async (p) => {
          const runs = await api.runs.listForProbe(p.id, 1).catch(() => [] as ProbeRun[]);
          return { id: p.id, run: runs[0] ?? null };
        }),
      );
      latestRunMap = Object.fromEntries(results.map((r) => [r.id, r.run]));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  });

  function onProbeCreated(probe: Probe) {
    probes = [probe, ...probes];
    latestRunMap = { [probe.id]: null, ...latestRunMap };
    showForm = false;
  }
</script>

<svelte:head>
  <title>ModelSentry — Probes</title>
</svelte:head>

<div class="page-header">
  <div>
    <h1 class="page-title">Probes</h1>
    <p class="page-subtitle">All configured LLM monitoring probes</p>
  </div>
  <div class="page-actions">
    <button class="btn btn-primary btn-sm" onclick={() => (showForm = !showForm)}>
      {showForm ? '✕ Cancel' : '+ New Probe'}
    </button>
  </div>
</div>

{#if showForm}
  <div class="section">
    <AddProbeForm oncreated={onProbeCreated} oncancel={() => (showForm = false)} />
  </div>
{/if}

{#if loading}
  <p class="loading-state">Loading…</p>
{:else if error}
  <div class="error-banner">Failed to load probes: {error}</div>
{:else if probes.length === 0 && !showForm}
  <div class="empty-state">
    No probes yet —
    <button class="btn btn-primary btn-sm" style="margin-left:var(--sp-2)" onclick={() => (showForm = true)}>
      + New Probe
    </button>
  </div>
{:else}
  <div class="card" style="padding: 0; overflow: hidden;">
    <ProbeTable {probes} {latestRunMap} />
  </div>
{/if}

