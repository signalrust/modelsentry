<script lang="ts">
  let { message = 'Loading…' }: { message?: string } = $props();
</script>

<div class="loading-state" role="status" aria-live="polite">
  <span class="spinner" aria-hidden="true"></span>
  <span>{message}</span>
</div>

<style>
  .loading-state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
  }
  .spinner {
    width: var(--sp-4);
    height: var(--sp-4);
    border: var(--border-width-sm) solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  /* Respect reduced-motion preferences. */
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
