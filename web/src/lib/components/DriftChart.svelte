<script lang="ts">
  import { onDestroy } from 'svelte';
  import { Chart, type ChartConfiguration } from 'chart.js';
  import 'chart.js/auto';
  import type { ProbeRun, BaselineSnapshot } from '$lib/types.js';

  let { probeId, runs, baseline }: {
    probeId: string;
    runs: ProbeRun[];
    baseline: BaselineSnapshot | null;
  } = $props();

  let canvas: HTMLCanvasElement = $state()!;
  let chart: Chart | null = null;

  // Resolve a theme CSS variable to its concrete value for Chart.js (which can't
  // read `var(--…)` directly). Falls back to transparent rather than a frozen
  // brand color, so nothing hardcoded leaks past the theme.
  function getCSSVar(name: string, fallback = 'transparent'): string {
    if (typeof document === 'undefined') return fallback;
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
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

    // Drift score = −log₁₀(combined p-value): higher ⇒ stronger evidence of
    // drift, monotone in significance and consistent across both test modes.
    const scoreValues = sorted.map((r) => r.drift_report?.statistic ?? null);
    // Significance line at −log₁₀(target_fpr): points above it crossed the
    // operator's false-positive rate and would alert. Derive from the most
    // recent report that carries a target FPR.
    const targetFpr = sorted
      .map((r) => r.drift_report?.target_fpr)
      .filter((v): v is number => typeof v === 'number' && v > 0)
      .at(-1);
    const sigLine = targetFpr ? -Math.log10(targetFpr) : null;
    const thresholdData = sorted.map(() => sigLine);

    const c1 = getCSSVar('--chart-1');
    const cDown = getCSSVar('--semantic-down');
    const textMuted = getCSSVar('--text-muted');
    const border = getCSSVar('--border');

    const config: ChartConfiguration = {
      type: 'line',
      data: {
        labels,
        datasets: [
          {
            label: 'Drift score (−log₁₀ p)',
            data: scoreValues,
            borderColor: c1,
            backgroundColor: c1 + '14',
            tension: 0.3,
            pointRadius: 4,
            fill: true,
            spanGaps: true,
          },
          {
            label: 'Alert threshold (target FPR)',
            data: thresholdData,
            borderColor: cDown,
            backgroundColor: 'transparent',
            borderDash: [6, 4],
            pointRadius: 0,
            borderWidth: 2,
            spanGaps: true,
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

  // `$effect` runs after mount and whenever `runs`/`baseline` change, so it
  // covers the initial build too — no separate `onMount` (which would build the
  // chart a second, redundant time on first render).
  $effect(() => {
    // Reference the reactive inputs so the effect re-runs when they change.
    void runs;
    void baseline;
    buildChart();
  });
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

