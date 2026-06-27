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
    gap: var(--sp-4);
  }

  h1 {
    font-size: var(--text-4xl);
    font-weight: 800;
    color: var(--semantic-down);
    margin: 0;
  }

  .error-message {
    font-size: var(--text-md);
    color: var(--text-muted);
    max-width: 28rem;
  }

  .btn-back {
    display: inline-block;
    margin-top: var(--sp-2);
    padding: var(--sp-2) var(--sp-4);
    background: var(--bg-input);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    text-decoration: none;
    font-size: var(--text-sm);
    transition: background var(--transition);
  }

  .btn-back:hover {
    background: var(--bg-elevated);
    border-color: var(--accent);
  }
</style>
