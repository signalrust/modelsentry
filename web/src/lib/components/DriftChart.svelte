<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Chart, type ChartConfiguration } from 'chart.js';
  import 'chart.js/auto';
  import type { ProbeRun, BaselineSnapshot } from '$lib/types.js';

  let { probeId, runs, baseline, klThreshold = 0.5 }: {
    probeId: string;
    runs: ProbeRun[];
    baseline: BaselineSnapshot | null;
    klThreshold?: number;
  } = $props();

  let canvas: HTMLCanvasElement = $state()!;
  let chart: Chart | null = null;

  function getCSSVar(name: string): string {
    if (typeof document === 'undefined') return '#3b82f6';
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || '#3b82f6';
  }

  function buildChart() {
    if (!canvas) return;

    const sorted = [...runs].sort(
      (a, b) => new Date(a.started_at).getTime() - new Date(b.started_at).getTime(),
    );

    const labels = sorted.map((r) =>
      new Date(r.started_at).toLocaleDateString(undefined, {
        month: 'short', day: 'numeric',
        hour: '2-digit', minute: '2-digit',
      }),
    );

    const klValues = sorted.map((r) => r.drift_report?.kl_divergence ?? null);
    const cosineValues = sorted.map((r) => r.drift_report?.cosine_distance ?? null);
    const thresholdData = sorted.map(() => klThreshold);

    const c1 = getCSSVar('--chart-1');
    const c4 = getCSSVar('--chart-4');
    const cDown = getCSSVar('--semantic-down');
    const textMuted = getCSSVar('--text-muted');
    const border = getCSSVar('--border');

    const config: ChartConfiguration = {
      type: 'line',
      data: {
        labels,
        datasets: [
          {
            label: 'KL Divergence',
            data: klValues,
            borderColor: c1,
            backgroundColor: c1 + '14',
            tension: 0.3,
            pointRadius: 4,
            fill: true,
            spanGaps: true,
          },
          {
            label: 'Cosine Distance',
            data: cosineValues,
            borderColor: c4,
            backgroundColor: 'transparent',
            tension: 0.3,
            pointRadius: 3,
            borderDash: [4, 3],
            spanGaps: true,
          },
          {
            label: 'KL Threshold',
            data: thresholdData,
            borderColor: cDown,
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
          legend: {
            position: 'top',
            labels: { color: textMuted, font: { size: 11, family: 'IBM Plex Mono, monospace' } },
          },
          tooltip: { enabled: true },
        },
        scales: {
          y: {
            beginAtZero: true,
            title: { display: false },
            grid: { color: border },
            ticks: { color: textMuted, font: { size: 10, family: 'IBM Plex Mono, monospace' } },
          },
          x: {
            ticks: {
              maxTicksLimit: 6,
              color: textMuted,
              font: { size: 10, family: 'IBM Plex Mono, monospace' },
            },
            grid: { color: border },
          },
        },
      },
    };

    if (chart) chart.destroy();
    chart = new Chart(canvas, config);
  }

  onMount(() => { buildChart(); });
  $effect(() => { runs; baseline; buildChart(); });
  onDestroy(() => { chart?.destroy(); });
</script>

<div class="chart-wrapper">
  <p class="chart-title">DRIFT — {probeId}</p>
  {#if runs.length === 0}
    <p class="empty-state" style="padding: var(--sp-6) 0">No runs yet.</p>
  {:else}
    <canvas class="chart-canvas" bind:this={canvas}></canvas>
  {/if}
</div>

