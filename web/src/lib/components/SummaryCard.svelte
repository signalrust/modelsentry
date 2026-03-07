<script lang="ts">
  let { title, value, status = 'neutral' }: {
    title: string;
    value: string | number;
    status?: 'ok' | 'warn' | 'error' | 'neutral';
  } = $props();
</script>

<div class="card" class:ok={status === 'ok'} class:warn={status === 'warn'} class:error={status === 'error'}>
  <p class="card-title">{title}</p>
  <p class="card-value">{value}</p>
</div>

<style>
  .card {
    background: #fff;
    border: 1px solid #e2e8f0;
    border-radius: 0.75rem;
    padding: 1.25rem 1.5rem;
    min-width: 160px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
    transition: background 0.2s;
  }

  .card.ok {
    background: #f0fdf4;
    border-color: #86efac;
  }

  .card.warn {
    background: #fefce8;
    border-color: #fde047;
  }

  .card.error {
    background: #fef2f2;
    border-color: #fca5a5;
  }

  .card-title {
    margin: 0 0 0.5rem;
    font-size: 0.8rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #64748b;
  }

  .card-value {
    margin: 0;
    font-size: 2rem;
    font-weight: 700;
    color: #0f172a;
    line-height: 1;
  }
</style>
