<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Chart, type ChartConfiguration } from 'chart.js';
  import 'chart.js/auto';
  import type { ProbeRun, BaselineSnapshot } from '$lib/types.js';

  /** The probe name shown in the chart title. */
  export let probeId: string;
  /** Pre-fetched recent runs for this probe, newest-first. */
  export let runs: ProbeRun[];
  /** Active baseline — used to draw the threshold reference line. */
  export let baseline: BaselineSnapshot | null;
  /** KL threshold from the alert rule (drawn as a red dashed line). */
  export let klThreshold: number = 0.5;

  // Svelte 4 binding target
  let canvas: HTMLCanvasElement;
  let chart: Chart | null = null;

  function buildChart() {
    if (!canvas) return;

    // Sort runs oldest-first for the chart x-axis.
    const sorted = [...runs].sort(
      (a, b) => new Date(a.started_at).getTime() - new Date(b.started_at).getTime(),
    );

    const labels = sorted.map((r) =>
      new Date(r.started_at).toLocaleDateString(undefined, {
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      }),
    );

    const klValues = sorted.map((r) => r.drift_report?.kl_divergence ?? null);
    const cosineValues = sorted.map((r) => r.drift_report?.cosine_distance ?? null);

    // Threshold reference line data (constant value across all points).
    const thresholdData = sorted.map(() => klThreshold);

    const config: ChartConfiguration = {
      type: 'line',
      data: {
        labels,
        datasets: [
          {
            label: 'KL Divergence',
            data: klValues,
            borderColor: '#3b82f6',
            backgroundColor: 'rgba(59,130,246,0.08)',
            tension: 0.3,
            pointRadius: 4,
            fill: true,
            spanGaps: true,
          },
          {
            label: 'Cosine Distance',
            data: cosineValues,
            borderColor: '#8b5cf6',
            backgroundColor: 'transparent',
            tension: 0.3,
            pointRadius: 3,
            borderDash: [4, 3],
            spanGaps: true,
          },
          {
            label: 'KL Threshold',
            data: thresholdData,
            borderColor: '#ef4444',
            backgroundColor: 'transparent',
            borderDash: [6, 4],
            pointRadius: 0,
            borderWidth: 2,
          },
        ],
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        interaction: { mode: 'index', intersect: false },
        plugins: {
          legend: { position: 'top' },
          tooltip: { enabled: true },
        },
        scales: {
          y: {
            beginAtZero: true,
            title: { display: true, text: 'Score' },
          },
          x: {
            ticks: { maxTicksLimit: 7 },
          },
        },
      },
    };

    if (chart) chart.destroy();
    chart = new Chart(canvas, config);
  }

  onMount(() => {
    buildChart();
  });

  // Rebuild whenever runs change.
  $: runs, baseline, buildChart();

  onDestroy(() => {
    chart?.destroy();
  });
</script>

<div class="chart-wrapper">
  <p class="chart-title">Drift — {probeId}</p>
  {#if runs.length === 0}
    <p class="empty">No runs yet.</p>
  {:else}
    <canvas bind:this={canvas}></canvas>
  {/if}
</div>

<style>
  .chart-wrapper {
    background: #fff;
    border: 1px solid #e2e8f0;
    border-radius: 0.75rem;
    padding: 1rem 1.25rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
  }

  .chart-title {
    margin: 0 0 0.75rem;
    font-size: 0.85rem;
    font-weight: 600;
    color: #475569;
  }

  canvas {
    height: 220px;
    width: 100%;
  }

  .empty {
    text-align: center;
    color: #94a3b8;
    font-size: 0.875rem;
    padding: 2rem 0;
    margin: 0;
  }
</style>
