<script lang="ts">
  import type { DriftReport, DriftLevel } from '$lib/types.js';

  let { report }: { report: DriftReport } = $props();

  const DRIFT_LABELS: Record<DriftLevel, string> = {
    none: 'None', low: 'Low', medium: 'Medium', high: 'High', critical: 'Critical',
  };

  const METHOD_LABELS: Record<string, string> = {
    per_prompt_conformal: 'Per-prompt conformal',
    pooled_two_sample: 'Pooled MMD/energy',
  };

  // Show enough significant figures that a tiny p-value doesn't render as "0".
  function fmtP(p: number): string {
    if (p === 0) return '0';
    if (p < 0.0001) return p.toExponential(2);
    return p.toFixed(4);
  }

  const methodLabel = $derived(METHOD_LABELS[report.method] ?? report.method);
  // Sort the per-prompt breakdown by strongest signal (lowest p) first.
  const sortedPrompts = $derived(
    [...report.per_prompt].sort((a, b) => a.p_value - b.p_value)
  );
</script>

<div class="drift-metrics">
  <div class="level-row">
    <span class="metric-label">Drift Level</span>
    <span class="badge" data-level={report.drift_level}>
      {DRIFT_LABELS[report.drift_level]}
    </span>
  </div>

  {#if report.interpretation}
    <p class="explanation" data-level={report.drift_level}>{report.interpretation}</p>
  {/if}

  <div class="verdict">
    <div class="metric-header">
      <span class="metric-label">Combined p-value</span>
      <span class="metric-value">{fmtP(report.combined_p_value)}</span>
    </div>
    <div class="metric-header">
      <span class="metric-label">Target FPR</span>
      <span class="metric-value">{fmtP(report.target_fpr)}</span>
    </div>
    <div class="metric-header">
      <span class="metric-label">Drift Score</span>
      <span class="metric-value">{report.statistic.toFixed(2)}</span>
    </div>
    <div class="metric-header">
      <span class="metric-label" title="Effect size: how far outputs moved, in standard deviations of the no-drift null (independent of baseline size).">Magnitude</span>
      <span class="metric-value">{report.effect_size.toFixed(1)} SD</span>
    </div>
    <div class="metric-header">
      <span class="metric-label">Method</span>
      <span class="metric-value method">{methodLabel}</span>
    </div>
  </div>

  {#if sortedPrompts.length > 0}
    <div class="per-prompt">
      <span class="metric-label">Per-prompt</span>
      {#each sortedPrompts as pp}
        <div class="metric-header prompt-row">
          <span class="prompt-idx">prompt #{pp.prompt_index}</span>
          <span class="metric-value">p = {fmtP(pp.p_value)} <span class="dim">· n={pp.n_baseline}</span></span>
        </div>
      {/each}
    </div>
  {/if}

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

  .explanation {
    margin: 0;
    padding: var(--sp-2) var(--sp-3);
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--text-secondary);
    background: var(--bg-input);
    border-left: 3px solid var(--border-strong);
    border-radius: var(--r-sm);
  }
  .explanation[data-level='high'],
  .explanation[data-level='critical'] {
    border-left-color: var(--semantic-down);
  }
  .explanation[data-level='low'],
  .explanation[data-level='medium'] {
    border-left-color: var(--accent);
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

  .verdict,
  .per-prompt {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .per-prompt {
    padding-top: var(--sp-2);
    border-top: 1px solid var(--border);
  }

  .metric-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--sp-3);
  }

  .metric-value {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
    font-family: var(--font-mono);
  }

  .metric-value.method {
    font-weight: 500;
    color: var(--text-secondary);
  }

  .prompt-idx {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    font-family: var(--font-mono);
  }

  .dim {
    color: var(--text-muted);
    font-weight: 400;
  }

  .computed-at {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-family: var(--font-mono);
    text-align: right;
    margin: 0;
  }
</style>

