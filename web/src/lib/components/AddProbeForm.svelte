<script lang="ts">
  import { api } from '$lib/api.js';
  import {
    CUSTOM_MODEL,
    DEFAULT_CRON,
    DEFAULT_MODELS,
    MODELS,
    OLLAMA_DEFAULT_BASE_URL,
    PROVIDER_KIND,
    PROVIDER_LABELS,
    type ProviderKindTag,
  } from '$lib/constants.js';
  import type { CreateProbeRequest, Probe, ProbePrompt, ProviderSpec, ProbeSchedule } from '$lib/types.js';

  let { oncreated, oncancel }: {
    oncreated: (probe: Probe) => void;
    oncancel: () => void;
  } = $props();

  // ── Form state ────────────────────────────────────────────────────────────
  let name = $state('');
  let providerKind: ProviderKindTag = $state(PROVIDER_KIND.OPEN_AI);
  let ollamaBaseUrl = $state(OLLAMA_DEFAULT_BASE_URL);
  // Azure: the resource endpoint and API key are configured server-side
  // ([providers.azure] + vault). The probe only chooses the chat deployment
  // (the model field) and, optionally, an embedding deployment for drift.
  let azureEmbeddingDeployment = $state('');
  let model = $state(DEFAULT_MODELS[PROVIDER_KIND.OPEN_AI]);
  let scheduleKind: 'every_minutes' | 'cron' = $state('every_minutes');
  let everyMinutes = $state(60);
  let cronExpression = $state(DEFAULT_CRON);
  let prompts: Array<{ text: string; expected_contains: string; expected_not_contains: string }> = $state([
    { text: '', expected_contains: '', expected_not_contains: '' },
  ]);
  let submitting = $state(false);
  let formError: string | null = $state(null);

  // Model selection: providers with a preset list use a <select> (with a
  // "Custom…" escape hatch); Ollama/Azure use free-text (open model space).
  const modelOptions = $derived(MODELS[providerKind]);
  const providerHasModelList = $derived(modelOptions.length > 0);
  let useCustomModel = $state(false);

  function onProviderKindChange() {
    model = DEFAULT_MODELS[providerKind];
    useCustomModel = false;
  }

  function onModelSelectChange() {
    // The "Custom…" sentinel switches the field to a free-text input.
    if (model === CUSTOM_MODEL) {
      useCustomModel = true;
      model = '';
    }
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
    if (providerKind === PROVIDER_KIND.OLLAMA && !ollamaBaseUrl.trim()) { formError = 'Ollama base URL is required.'; return; }
    const validPrompts = prompts.filter((p) => p.text.trim());
    if (validPrompts.length === 0) { formError = 'At least one prompt is required.'; return; }

    submitting = true;
    try {
      const trimmedModel = model.trim();
      let provider: ProviderSpec;
      if (providerKind === PROVIDER_KIND.OPEN_AI) provider = { kind: PROVIDER_KIND.OPEN_AI, model: trimmedModel };
      else if (providerKind === PROVIDER_KIND.ANTHROPIC) provider = { kind: PROVIDER_KIND.ANTHROPIC, model: trimmedModel };
      else if (providerKind === PROVIDER_KIND.OLLAMA) provider = { kind: PROVIDER_KIND.OLLAMA, model: trimmedModel, base_url: ollamaBaseUrl.trim() };
      else provider = { kind: PROVIDER_KIND.AZURE, chat_deployment: trimmedModel, embedding_deployment: azureEmbeddingDeployment.trim() || null };

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
        prompts: builtPrompts,
        schedule,
      };

      const probe = await api.probes.create(body);
      oncreated(probe);
    } catch (e) {
      formError = e instanceof Error ? e.message : String(e);
    } finally {
      submitting = false;
    }
  }
</script>

<div class="card form-card">
  <h2 class="form-title">New Probe</h2>

  {#if formError}
    <div class="error-banner form-error">{formError}</div>
  {/if}

  <form onsubmit={(e: Event) => { e.preventDefault(); void handleSubmit(); }}>
    <!-- Name -->
    <div class="field">
      <label for="probe-name">Name</label>
      <input id="probe-name" type="text" bind:value={name} placeholder="e.g. production-gpt4" required />
    </div>

    <!-- Provider -->
    <div class="field">
      <label for="probe-provider">Provider</label>
      <select id="probe-provider" bind:value={providerKind} onchange={onProviderKindChange}>
        <option value={PROVIDER_KIND.OPEN_AI}>{PROVIDER_LABELS[PROVIDER_KIND.OPEN_AI]}</option>
        <option value={PROVIDER_KIND.ANTHROPIC}>{PROVIDER_LABELS[PROVIDER_KIND.ANTHROPIC]}</option>
        <option value={PROVIDER_KIND.OLLAMA}>{PROVIDER_LABELS[PROVIDER_KIND.OLLAMA]} (self-hosted)</option>
        <option value={PROVIDER_KIND.AZURE}>{PROVIDER_LABELS[PROVIDER_KIND.AZURE]}</option>
      </select>
    </div>

    {#if providerKind === PROVIDER_KIND.OLLAMA}
      <div class="field">
        <label for="ollama-url">Ollama Base URL</label>
        <input id="ollama-url" type="text" bind:value={ollamaBaseUrl} placeholder={OLLAMA_DEFAULT_BASE_URL} />
      </div>
    {/if}

    {#if providerKind === PROVIDER_KIND.AZURE}
      <div class="field">
        <label for="azure-embed-deployment">Embedding deployment <span class="field-hint">(optional)</span></label>
        <input id="azure-embed-deployment" type="text" bind:value={azureEmbeddingDeployment} placeholder="text-embedding-3-small deployment" />
        <p class="field-hint">
          The resource endpoint and API key are configured server-side
          (<code>[providers.azure]</code> + vault). The Model field below is the
          chat deployment. Leave embedding deployment blank to run completions-only
          (no drift detection).
        </p>
      </div>
    {/if}

    <!-- Model -->
    <div class="field">
      <label for="probe-model">Model</label>
      {#if providerHasModelList && !useCustomModel}
        <select id="probe-model" bind:value={model} onchange={onModelSelectChange}>
          {#each modelOptions as m}
            <option value={m}>{m}</option>
          {/each}
          <option value={CUSTOM_MODEL}>Custom…</option>
        </select>
      {:else}
        <input id="probe-model" type="text" bind:value={model} placeholder="model name" required />
        {#if providerHasModelList}
          <button
            type="button"
            class="btn-link"
            onclick={() => { useCustomModel = false; model = DEFAULT_MODELS[providerKind]; }}
          >← choose from list</button>
        {/if}
      {/if}
    </div>

    <!-- Schedule -->
    <div class="field">
      <p class="field-section-label">Schedule</p>
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
        <input id="probe-minutes" type="number" min="1" max="10080" bind:value={everyMinutes} class="input-narrow" />
        <span class="field-hint">minutes</span>
      </div>
    {:else}
      <div class="field">
        <label for="probe-cron">Cron expression
          <span class="field-hint"> (5-field: min hour dom month dow)</span>
        </label>
        <input id="probe-cron" type="text" bind:value={cronExpression} placeholder="0 * * * *" />
      </div>
    {/if}

    <!-- Prompts -->
    <div class="field">
      <div class="prompts-header">
        <p class="field-section-label">Prompts</p>
        <button type="button" class="btn-add-prompt" onclick={addPrompt}>+ Add prompt</button>
      </div>

      {#each prompts as prompt, i}
        <div class="prompt-block">
          <div class="prompt-block-header">
            <span class="prompt-num">Prompt {i + 1}</span>
            {#if prompts.length > 1}
              <button type="button" class="btn-remove" onclick={() => removePrompt(i)}>Remove</button>
            {/if}
          </div>
          <div class="field">
            <textarea
              rows="3"
              bind:value={prompt.text}
              placeholder="Enter the prompt text sent to the model…"
            ></textarea>
          </div>
          <div class="field-row">
            <div class="field">
              <label for="must-contains-{i}">Must contain <span class="field-hint">(optional)</span></label>
              <input id="must-contains-{i}" type="text" bind:value={prompt.expected_contains} placeholder="expected substring" />
            </div>
            <div class="field">
              <label for="must-not-contains-{i}">Must not contain <span class="field-hint">(optional)</span></label>
              <input id="must-not-contains-{i}" type="text" bind:value={prompt.expected_not_contains} placeholder="forbidden substring" />
            </div>
          </div>
        </div>
      {/each}
    </div>

    <!-- Actions -->
    <div class="form-actions">
      <button type="button" class="btn" onclick={() => oncancel()} disabled={submitting}>
        Cancel
      </button>
      <button type="submit" class="btn btn-primary" disabled={submitting}>
        {submitting ? 'Creating…' : 'Create Probe'}
      </button>
    </div>
  </form>
</div>

<style>
  .form-card { margin-bottom: var(--sp-6); }

  .form-title {
    font-size: var(--text-md);
    font-weight: 700;
    margin-bottom: var(--sp-4);
    color: var(--text-primary);
    font-family: var(--font-display);
  }

  .form-error { margin-bottom: var(--sp-4); }

  .field-inline {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .field-inline label { margin-bottom: 0; white-space: nowrap; }
  /* Specificity (0,2,0) beats the global `.field input` (0,1,1) — no !important. */
  .field-inline .input-narrow { width: 7rem; }

  .btn-link {
    margin-top: var(--sp-2);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--accent);
    background: none;
    border: none;
    cursor: pointer;
    font-family: var(--font-mono);
    padding: 0;
    transition: opacity var(--transition);
  }
  .btn-link:hover { opacity: 0.7; }

  .prompts-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--sp-2);
  }
  .prompts-header .field-section-label { margin-bottom: 0; }

  .field-section-label {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0 0 var(--sp-1);
    font-family: var(--font-display);
  }

  .btn-add-prompt {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--accent);
    background: none;
    border: none;
    cursor: pointer;
    font-family: var(--font-mono);
    padding: 0;
    transition: opacity var(--transition);
  }
  .btn-add-prompt:hover { opacity: 0.7; }

  .prompt-block {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: var(--sp-3);
    margin-bottom: var(--sp-3);
  }
  .prompt-block .field { margin-bottom: var(--sp-2); }
  .prompt-block .field:last-child { margin-bottom: 0; }

  .prompt-block-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: var(--sp-2);
  }
  .prompt-num {
    font-size: var(--text-xs);
    font-weight: 700;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-family: var(--font-mono);
  }
  .btn-remove {
    font-size: var(--text-xs);
    color: var(--semantic-down);
    background: none;
    border: none;
    cursor: pointer;
    font-family: var(--font-mono);
    font-weight: 600;
    padding: 0;
  }
  .btn-remove:hover { text-decoration: underline; }

  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-2);
    margin-top: var(--sp-6);
    padding-top: var(--sp-4);
    border-top: 1px solid var(--border);
  }
</style>
