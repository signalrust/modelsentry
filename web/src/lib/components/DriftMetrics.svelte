<script lang="ts">
  import type { DriftReport, DriftLevel } from '$lib/types.js';

  let { report }: { report: DriftReport } = $props();

  const DRIFT_LABELS: Record<DriftLevel, string> = {
    none: 'None', low: 'Low', medium: 'Medium', high: 'High', critical: 'Critical',
  };

  function meterPct(value: number, max: number): number {
    return Math.min(100, Math.round((value / max) * 100));
  }
</script>

<div class="drift-metrics">
  <div class="level-row">
    <span class="metric-label">Drift Level</span>
    <span class="badge" data-level={report.drift_level}>
      {DRIFT_LABELS[report.drift_level]}
    </span>
  </div>

  {#each [
    { label: 'KL Divergence',  value: report.kl_divergence,           max: 2 },
    { label: 'Cosine Distance', value: report.cosine_distance,         max: 1 },
    { label: 'Entropy Delta',   value: Math.abs(report.output_entropy_delta), max: 2 },
  ] as metric}
    <div class="metric">
      <div class="metric-header">
        <span class="metric-label">{metric.label}</span>
        <span class="metric-value">{metric.value.toFixed(4)}</span>
      </div>
      <div class="meter">
        <div
          class="meter-fill"
          data-level={report.drift_level}
          style="width: {meterPct(metric.value, metric.max)}%"
        ></div>
      </div>
    </div>
  {/each}

  <p class="computed-at">computed {new Date(report.computed_at).toLocaleString()}</p>
</div>

<style>
  .drift-metrics {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .level-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }

  .metric-label {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.07em;
    font-family: var(--font-mono);
    white-space: nowrap;
  }

  .metric {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .metric-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .metric-value {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
    font-family: var(--font-mono);
  }

  .computed-at {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-family: var(--font-mono);
    text-align: right;
    margin: 0;
  }
</style>

