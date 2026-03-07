<script lang="ts">
  import { page } from '$app/state';
</script>

<svelte:head>
  <title>ModelSentry — Error {page.status}</title>
</svelte:head>

<main>
  <div class="error-container">
    <h1>{page.status}</h1>
    <p class="error-message">{page.error?.message ?? 'Something went wrong'}</p>
    <a class="btn-back" href="/">← Back to Dashboard</a>
  </div>
</main>

<style>
  .error-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: 60vh;
    text-align: center;
    gap: 1rem;
  }

  h1 {
    font-size: 4rem;
    font-weight: 800;
    color: var(--color-error, #e53e3e);
    margin: 0;
  }

  .error-message {
    font-size: 1.125rem;
    color: var(--color-muted, #888);
    max-width: 28rem;
  }

  .btn-back {
    display: inline-block;
    margin-top: 0.5rem;
    padding: 0.5rem 1.25rem;
    background: var(--color-surface, #1e1e2e);
    color: var(--color-text, #e0e0e0);
    border: 1px solid var(--color-border, #333);
    border-radius: 6px;
    text-decoration: none;
    font-size: 0.875rem;
    transition: background 0.15s;
  }

  .btn-back:hover {
    background: var(--color-hover, #2a2a3e);
  }
</style>
