<script lang="ts">
  let { title, value, status = 'neutral' }: {
    title: string;
    value: string | number;
    status?: 'ok' | 'warn' | 'error' | 'neutral';
  } = $props();
</script>

<div class="kpi" class:ok={status === 'ok'} class:warn={status === 'warn'} class:error={status === 'error'}>
  <p class="kpi-label">{title}</p>
  <p class="kpi-value">{value}</p>
</div>

