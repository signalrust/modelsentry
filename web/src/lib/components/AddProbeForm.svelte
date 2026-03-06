<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { api } from '$lib/api.js';
  import type { CreateProbeRequest, Probe, ProbePrompt, ProviderKind, ProbeSchedule } from '$lib/types.js';

  const dispatch = createEventDispatcher<{ created: Probe; cancel: void }>();

  // ── Form state ────────────────────────────────────────────────────────────
  let name = '';
  let providerKind: 'open_ai' | 'anthropic' | 'ollama' | 'azure_open_ai' = 'open_ai';
  let ollamaBaseUrl = 'http://localhost:11434';
  let azureEndpoint = '';
  let azureDeployment = '';
  let model = 'gpt-4o';
  let scheduleKind: 'every_minutes' | 'cron' = 'every_minutes';
  let everyMinutes = 60;
  let cronExpression = '0 * * * *';
  let prompts: Array<{ text: string; expected_contains: string; expected_not_contains: string }> = [
    { text: '', expected_contains: '', expected_not_contains: '' },
  ];
  let submitting = false;
  let formError: string | null = null;

  const defaultModels: Record<string, string> = {
    open_ai: 'gpt-4o',
    anthropic: 'claude-3-7-sonnet-20250219',
    ollama: 'llama3',
    azure_open_ai: 'gpt-4o',
  };

  function onProviderKindChange() {
    model = defaultModels[providerKind] ?? '';
  }

  function addPrompt() {
    prompts = [...prompts, { text: '', expected_contains: '', expected_not_contains: '' }];
  }

  function removePrompt(i: number) {
    if (prompts.length > 1) {
      prompts = prompts.filter((_, idx) => idx !== i);
    }
  }

  async function handleSubmit() {
    formError = null;
    if (!name.trim()) { formError = 'Name is required.'; return; }
    if (!model.trim()) { formError = 'Model is required.'; return; }
    if (providerKind === 'ollama' && !ollamaBaseUrl.trim()) { formError = 'Ollama base URL is required.'; return; }
    if (providerKind === 'azure_open_ai' && (!azureEndpoint.trim() || !azureDeployment.trim())) {
      formError = 'Azure endpoint and deployment are required.'; return;
    }
    const validPrompts = prompts.filter((p) => p.text.trim());
    if (validPrompts.length === 0) { formError = 'At least one prompt is required.'; return; }

    submitting = true;
    try {
      let provider: ProviderKind;
      if (providerKind === 'open_ai') provider = { kind: 'open_ai' };
      else if (providerKind === 'anthropic') provider = { kind: 'anthropic' };
      else if (providerKind === 'ollama') provider = { kind: 'ollama', base_url: ollamaBaseUrl.trim() };
      else provider = { kind: 'azure_open_ai', endpoint: azureEndpoint.trim(), deployment: azureDeployment.trim() };

      let schedule: ProbeSchedule;
      if (scheduleKind === 'every_minutes') {
        schedule = { kind: 'every_minutes', minutes: Math.max(1, Math.floor(everyMinutes)) };
      } else {
        schedule = { kind: 'cron', expression: cronExpression.trim() };
      }

      const builtPrompts: ProbePrompt[] = validPrompts.map((p) => ({
        id: crypto.randomUUID(),
        text: p.text.trim(),
        expected_contains: p.expected_contains.trim() || null,
        expected_not_contains: p.expected_not_contains.trim() || null,
      }));

      const body: CreateProbeRequest = {
        name: name.trim(),
        provider,
        model: model.trim(),
        prompts: builtPrompts,
        schedule,
      };

      const probe = await api.probes.create(body);
      dispatch('created', probe);
    } catch (e) {
      formError = e instanceof Error ? e.message : String(e);
    } finally {
      submitting = false;
    }
  }
</script>

<div class="form-card">
  <h2 class="form-title">New Probe</h2>

  {#if formError}
    <p class="error-banner">{formError}</p>
  {/if}

  <form on:submit|preventDefault={handleSubmit}>
    <!-- Name -->
    <div class="field">
      <label for="probe-name">Name</label>
      <input id="probe-name" type="text" bind:value={name} placeholder="e.g. production-gpt4" required />
    </div>

    <!-- Provider -->
    <div class="field">
      <label for="probe-provider">Provider</label>
      <select id="probe-provider" bind:value={providerKind} on:change={onProviderKindChange}>
        <option value="open_ai">OpenAI</option>
        <option value="anthropic">Anthropic</option>
        <option value="ollama">Ollama (self-hosted)</option>
        <option value="azure_open_ai">Azure OpenAI</option>
      </select>
    </div>

    {#if providerKind === 'ollama'}
      <div class="field">
        <label for="ollama-url">Ollama Base URL</label>
        <input id="ollama-url" type="text" bind:value={ollamaBaseUrl} placeholder="http://localhost:11434" />
      </div>
    {/if}

    {#if providerKind === 'azure_open_ai'}
      <div class="field-row">
        <div class="field">
          <label for="azure-endpoint">Endpoint</label>
          <input id="azure-endpoint" type="text" bind:value={azureEndpoint} placeholder="https://my-resource.openai.azure.com" />
        </div>
        <div class="field">
          <label for="azure-deployment">Deployment</label>
          <input id="azure-deployment" type="text" bind:value={azureDeployment} placeholder="gpt-4o" />
        </div>
      </div>
    {/if}

    <!-- Model -->
    <div class="field">
      <label for="probe-model">Model</label>
      <input id="probe-model" type="text" bind:value={model} placeholder="e.g. gpt-4o" required />
    </div>

    <!-- Schedule -->
    <div class="field">
      <label>Schedule</label>
      <div class="radio-row">
        <label class="radio-label">
          <input type="radio" bind:group={scheduleKind} value="every_minutes" />
          Every N minutes
        </label>
        <label class="radio-label">
          <input type="radio" bind:group={scheduleKind} value="cron" />
          Cron expression
        </label>
      </div>
    </div>

    {#if scheduleKind === 'every_minutes'}
      <div class="field field-inline">
        <label for="probe-minutes">Run every</label>
        <input id="probe-minutes" type="number" min="1" max="10080" bind:value={everyMinutes} style="width:6rem" />
        <span class="unit">minutes</span>
      </div>
    {:else}
      <div class="field">
        <label for="probe-cron">Cron expression <span class="hint">(5-field: min hour dom month dow)</span></label>
        <input id="probe-cron" type="text" bind:value={cronExpression} placeholder="0 * * * *" />
      </div>
    {/if}

    <!-- Prompts -->
    <div class="field">
      <div class="section-header">
        <label>Prompts</label>
        <button type="button" class="btn-add-prompt" on:click={addPrompt}>+ Add prompt</button>
      </div>

      {#each prompts as prompt, i}
        <div class="prompt-block">
          <div class="prompt-header">
            <span class="prompt-num">Prompt {i + 1}</span>
            {#if prompts.length > 1}
              <button type="button" class="btn-remove" on:click={() => removePrompt(i)}>Remove</button>
            {/if}
          </div>
          <textarea
            rows="3"
            bind:value={prompt.text}
            placeholder="Enter the prompt text sent to the model…"
          ></textarea>
          <div class="field-row expect-row">
            <div class="field small">
              <label>Must contain (optional)</label>
              <input type="text" bind:value={prompt.expected_contains} placeholder="expected substring" />
            </div>
            <div class="field small">
              <label>Must not contain (optional)</label>
              <input type="text" bind:value={prompt.expected_not_contains} placeholder="forbidden substring" />
            </div>
          </div>
        </div>
      {/each}
    </div>

    <!-- Actions -->
    <div class="form-actions">
      <button type="button" class="btn-secondary" on:click={() => dispatch('cancel')} disabled={submitting}>
        Cancel
      </button>
      <button type="submit" class="btn-primary" disabled={submitting}>
        {submitting ? 'Creating…' : 'Create Probe'}
      </button>
    </div>
  </form>
</div>

<style>
  .form-card {
    background: #fff;
    border: 1px solid #e2e8f0;
    border-radius: 0.75rem;
    padding: 1.75rem;
    margin-bottom: 2rem;
  }

  .form-title {
    margin: 0 0 1.25rem;
    font-size: 1.1rem;
    font-weight: 700;
    color: #0f172a;
  }

  .error-banner {
    background: #fef2f2;
    border: 1px solid #fecaca;
    color: #dc2626;
    border-radius: 0.4rem;
    padding: 0.6rem 0.9rem;
    font-size: 0.875rem;
    margin-bottom: 1rem;
  }

  .field {
    margin-bottom: 1rem;
  }

  .field label {
    display: block;
    font-size: 0.8rem;
    font-weight: 600;
    color: #475569;
    margin-bottom: 0.3rem;
  }

  .field input[type='text'],
  .field input[type='number'],
  .field select,
  .field textarea {
    width: 100%;
    box-sizing: border-box;
    padding: 0.45rem 0.7rem;
    border: 1px solid #cbd5e1;
    border-radius: 0.4rem;
    font-size: 0.875rem;
    color: #0f172a;
    background: #f8fafc;
  }

  .field input:focus,
  .field select:focus,
  .field textarea:focus {
    outline: 2px solid #6366f1;
    outline-offset: -1px;
    background: #fff;
  }

  .field-row {
    display: flex;
    gap: 0.75rem;
  }

  .field-row .field {
    flex: 1;
  }

  .field.small input {
    font-size: 0.8rem;
  }

  .field-inline {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .field-inline label {
    margin: 0;
    white-space: nowrap;
  }

  .unit {
    font-size: 0.875rem;
    color: #64748b;
  }

  .radio-row {
    display: flex;
    gap: 1.5rem;
    margin-top: 0.2rem;
  }

  .radio-label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.875rem;
    font-weight: 500;
    color: #334155;
    cursor: pointer;
  }

  .hint {
    font-weight: 400;
    color: #94a3b8;
    font-size: 0.75rem;
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.5rem;
  }

  .section-header label {
    margin: 0;
  }

  .btn-add-prompt {
    font-size: 0.8rem;
    font-weight: 600;
    color: #6366f1;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
  }

  .btn-add-prompt:hover {
    text-decoration: underline;
  }

  .prompt-block {
    background: #f8fafc;
    border: 1px solid #e2e8f0;
    border-radius: 0.5rem;
    padding: 0.75rem;
    margin-bottom: 0.75rem;
  }

  .prompt-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.5rem;
  }

  .prompt-num {
    font-size: 0.78rem;
    font-weight: 600;
    color: #64748b;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .btn-remove {
    font-size: 0.78rem;
    color: #ef4444;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    font-weight: 600;
  }

  .btn-remove:hover {
    text-decoration: underline;
  }

  .prompt-block textarea {
    width: 100%;
    box-sizing: border-box;
    padding: 0.45rem 0.7rem;
    border: 1px solid #cbd5e1;
    border-radius: 0.4rem;
    font-size: 0.875rem;
    color: #0f172a;
    background: #fff;
    resize: vertical;
    margin-bottom: 0.5rem;
  }

  .prompt-block textarea:focus {
    outline: 2px solid #6366f1;
    outline-offset: -1px;
  }

  .expect-row {
    margin-bottom: 0;
  }

  .expect-row .field {
    margin-bottom: 0;
  }

  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
    margin-top: 1.5rem;
    padding-top: 1rem;
    border-top: 1px solid #f1f5f9;
  }

  .btn-primary {
    padding: 0.5rem 1.25rem;
    background: #6366f1;
    color: #fff;
    border: none;
    border-radius: 0.4rem;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
  }

  .btn-primary:hover:not(:disabled) {
    background: #4f46e5;
  }

  .btn-primary:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .btn-secondary {
    padding: 0.5rem 1.25rem;
    background: #f1f5f9;
    color: #334155;
    border: 1px solid #e2e8f0;
    border-radius: 0.4rem;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
  }

  .btn-secondary:hover:not(:disabled) {
    background: #e2e8f0;
  }
</style>
