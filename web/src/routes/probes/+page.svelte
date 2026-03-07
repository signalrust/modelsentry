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

      // Fetch the latest run for each probe in parallel.
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

<main>
  <header>
    <div class="header-row">
      <div>
        <h1>Probes</h1>
        <p class="subtitle">All configured LLM probes</p>
      </div>
      <div class="header-actions">
        <button class="btn-new" onclick={() => (showForm = !showForm)}>
          {showForm ? '✕ Cancel' : '+ New Probe'}
        </button>
        <a class="btn-back" href="/">← Dashboard</a>
      </div>
    </div>
  </header>

  {#if showForm}
    <AddProbeForm oncreated={onProbeCreated} oncancel={() => (showForm = false)} />
  {/if}

  {#if loading}
    <p class="status-msg">Loading…</p>
  {:else if error}
    <p class="error-msg">Failed to load probes: {error}</p>
  {:else if probes.length === 0 && !showForm}
    <p class="status-msg">No probes yet — click <strong>+ New Probe</strong> to add one.</p>
  {:else}
    <ProbeTable {probes} {latestRunMap} />
  {/if}
</main>

<style>
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

  h1 {
    margin: 0;
    font-size: 1.5rem;
    font-weight: 800;
    color: #0f172a;
  }

  .subtitle {
    margin: 0.2rem 0 0;
    color: #64748b;
    font-size: 0.875rem;
  }

  .header-actions {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    flex-wrap: wrap;
  }

  .btn-new {
    display: inline-block;
    padding: 0.45rem 1rem;
    background: #6366f1;
    color: #fff;
    border: none;
    border-radius: 0.4rem;
    font-size: 0.85rem;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }

  .btn-new:hover {
    background: #4f46e5;
  }

  .btn-back {
    display: inline-block;
    padding: 0.45rem 1rem;
    background: #f1f5f9;
    color: #334155;
    border-radius: 0.4rem;
    font-size: 0.85rem;
    font-weight: 600;
    text-decoration: none;
    white-space: nowrap;
    border: 1px solid #e2e8f0;
  }

  .btn-back:hover {
    background: #e2e8f0;
  }

  .status-msg,
  .error-msg {
    text-align: center;
    padding: 3rem 0;
    color: #64748b;
  }

  .error-msg {
    color: #dc2626;
  }
</style>
