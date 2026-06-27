<script lang="ts">
  import { auth } from '$lib/auth.svelte.js';

  let {
    message,
    onretry,
  }: { message: string; onretry?: () => void } = $props();
</script>

<div class="error-banner error-state" role="alert">
  <span class="error-text">{message}</span>
  <div class="error-actions">
    {#if auth.unauthorized}
      <span class="auth-hint">Set your API key (🔓, top-right), then retry.</span>
    {/if}
    {#if onretry}
      <button class="btn btn-sm" onclick={onretry}>Retry</button>
    {/if}
  </div>
</div>

<style>
  .error-state {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    flex-wrap: wrap;
  }
  .error-text {
    flex: 1;
    min-width: 0;
  }
  .error-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .auth-hint {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
</style>
