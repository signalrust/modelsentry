<script lang="ts">
  import { auth } from '$lib/auth.svelte.js';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  let draft = $state(auth.key ?? '');

  // Re-seed the input from the stored key each time the dialog opens.
  $effect(() => {
    if (open) draft = auth.key ?? '';
  });

  function save() {
    auth.set(draft);
    open = false;
  }

  function clear() {
    auth.set(null);
    draft = '';
    open = false;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') open = false;
    if (e.key === 'Enter') save();
  }
</script>

{#if open}
  <div class="dialog-overlay" role="presentation" onclick={() => (open = false)}></div>
  <div
    class="dialog-card"
    role="dialog"
    aria-modal="true"
    aria-label="API key settings"
  >
    <h2 class="dialog-title">API Key</h2>
    <p class="dialog-help">
      Required when the daemon is started with <code>[auth] enabled = true</code>.
      Sent as a <code>Bearer</code> token and stored locally in this browser.
    </p>

    {#if auth.unauthorized}
      <div class="error-banner" style="margin-bottom: var(--sp-3)">
        The daemon rejected the current key (401). Enter a valid key to continue.
      </div>
    {/if}

    <!-- svelte-ignore a11y_autofocus -->
    <input
      class="key-input"
      type="password"
      placeholder="your-secret-key"
      autocomplete="off"
      autofocus
      bind:value={draft}
      onkeydown={onKeydown}
    />

    <div class="dialog-actions">
      {#if auth.key}
        <button class="btn btn-danger btn-sm" onclick={clear}>Clear</button>
      {/if}
      <span class="spacer"></span>
      <button class="btn btn-sm" onclick={() => (open = false)}>Cancel</button>
      <button class="btn btn-primary btn-sm" onclick={save}>Save</button>
    </div>
  </div>
{/if}

<style>
  .dialog-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 200;
  }
  .dialog-card {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(440px, calc(100vw - 2 * var(--sp-4)));
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-md);
    padding: var(--sp-5);
    z-index: 201;
    box-shadow: var(--shadow-glow);
  }
  .dialog-title {
    margin: 0 0 var(--sp-2);
  }
  .dialog-help {
    color: var(--text-muted);
    font-size: var(--text-sm);
    line-height: 1.5;
    margin-bottom: var(--sp-4);
  }
  .dialog-help code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-primary);
  }
  .key-input {
    width: 100%;
    padding: var(--sp-2) var(--sp-3);
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .key-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .dialog-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-top: var(--sp-4);
  }
  .spacer {
    flex: 1;
  }
</style>
