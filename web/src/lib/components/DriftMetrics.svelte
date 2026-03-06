<script lang="ts">
  import type { DriftReport, DriftLevel } from '$lib/types.js';

  export let report: DriftReport;

  const DRIFT_LABELS: Record<DriftLevel, string> = {
    none: 'None',
    low: 'Low',
    medium: 'Medium',
    high: 'High',
    critical: 'Critical',
  };

  /** Width percentage for the mini meter bar (0–100). */
  function meterPct(value: number, max: number): number {
    return Math.min(100, Math.round((value / max) * 100));
  }
</script>

<div class="drift-metrics">
  <div class="level-row">
    <span class="label">Drift Level</span>
    <span class="badge" data-level={report.drift_level}>
      {DRIFT_LABELS[report.drift_level]}
    </span>
  </div>

  <div class="metric">
    <div class="metric-header">
      <span class="label">KL Divergence</span>
      <span class="value">{report.kl_divergence.toFixed(4)}</span>
    </div>
    <div class="meter">
      <div
        class="meter-fill"
        data-level={report.drift_level}
        style="width: {meterPct(report.kl_divergence, 2)}%"
      ></div>
    </div>
  </div>

  <div class="metric">
    <div class="metric-header">
      <span class="label">Cosine Distance</span>
      <span class="value">{report.cosine_distance.toFixed(4)}</span>
    </div>
    <div class="meter">
      <div
        class="meter-fill"
        data-level={report.drift_level}
        style="width: {meterPct(report.cosine_distance, 1)}%"
      ></div>
    </div>
  </div>

  <div class="metric">
    <div class="metric-header">
      <span class="label">Entropy Delta</span>
      <span class="value">{report.output_entropy_delta.toFixed(4)}</span>
    </div>
    <div class="meter">
      <div
        class="meter-fill"
        data-level={report.drift_level}
        style="width: {meterPct(Math.abs(report.output_entropy_delta), 2)}%"
      ></div>
    </div>
  </div>

  <p class="computed-at">
    Computed {new Date(report.computed_at).toLocaleString()}
  </p>
</div>

<style>
  .drift-metrics {
    background: #fff;
    border: 1px solid #e2e8f0;
    border-radius: 0.75rem;
    padding: 1rem 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .level-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .label {
    font-size: 0.8rem;
    font-weight: 600;
    color: #64748b;
    white-space: nowrap;
  }

  .badge {
    display: inline-block;
    padding: 0.2rem 0.75rem;
    border-radius: 999px;
    font-size: 0.75rem;
    font-weight: 700;
    text-transform: uppercase;
    background: #e2e8f0;
    color: #334155;
  }

  .badge[data-level='low']      { background: #dbeafe; color: #2563eb; }
  .badge[data-level='medium']   { background: #fef3c7; color: #d97706; }
  .badge[data-level='high']     { background: #fee2e2; color: #dc2626; }
  .badge[data-level='critical'] { background: #dc2626; color: #fff; }

  /* Metric row */
  .metric {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .metric-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .value {
    font-size: 0.85rem;
    font-weight: 600;
    color: #0f172a;
    font-variant-numeric: tabular-nums;
  }

  /* Progress bar */
  .meter {
    height: 6px;
    border-radius: 999px;
    background: #f1f5f9;
    overflow: hidden;
  }

  .meter-fill {
    height: 100%;
    border-radius: 999px;
    background: #94a3b8;
    transition: width 0.3s ease;
  }

  .meter-fill[data-level='low']      { background: #60a5fa; }
  .meter-fill[data-level='medium']   { background: #fbbf24; }
  .meter-fill[data-level='high']     { background: #f87171; }
  .meter-fill[data-level='critical'] { background: #dc2626; }

  .computed-at {
    margin: 0;
    font-size: 0.72rem;
    color: #94a3b8;
    text-align: right;
  }
</style>
