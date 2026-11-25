<script lang="ts">
  import Dialog from '../../ui/Dialog.svelte';
  import Select from '../../ui/Select.svelte';
  import { errorText, t } from '../../i18n';
  import { aiProviderModels, aiProviderTest, aiRevealKey, aiSetKey, type Preferences, type ProviderFormat, type ProviderPrefs } from '../../ipc';
  import { tokens } from '../../ai/quota';
  import { settingsError } from '../edit.svelte';
  import { addModel, CUSTOM_PRESET, EFFORT_LEVELS, fromPreset, isKeylessProvider, mergeListed, PRESETS, providerId, providerModels, type ProviderPreset } from '../providers';
  import AiKeyField from './AiKeyField.svelte';

  interface Props {
    provider: ProviderPrefs | null;
    taken: string[];
    stored: boolean;
    updatePrefs: (change: (draft: Preferences) => void) => Promise<void>;
    onclose: () => void;
    onsaved: () => void;
  }

  let { provider, taken, stored, updatePrefs, onclose, onsaved }: Props = $props();
  function initialDraft(): ProviderPrefs {
    return provider ? structuredClone($state.snapshot(provider)) : fromPreset(CUSTOM_PRESET, taken);
  }

  let draft = $state(initialDraft());
  let persistedId = $state<string | null>(null);
  let presetId = $state(CUSTOM_PRESET.id);
  let key = $state('');
  let keyField: AiKeyField | undefined = $state();
  function initialModels(): string[] {
    return [draft.model, ...draft.models].filter(Boolean);
  }
  let picked = $state(initialModels());
  let listed = $state(initialModels());
  let manual = $state<string[]>([]);
  let modelId = $state('');
  let query = $state('');
  let saving = $state(false);
  let error = $state('');
  let fetching = $state(false);
  let fetchError = $state('');
  let testing = $state(false);
  let testResult = $state<{ ok: boolean; text: string } | null>(null);
  const chosen = $derived(providerModels(picked, draft.model));
  const canRequest = $derived(!!draft.base_url.trim());
  const canSave = $derived(canRequest && !!chosen.model && !!draft.name.trim());
  const shownModels = $derived(listed.filter((model) => model.toLowerCase().includes(query.trim().toLowerCase())));
  const keyless = $derived(isKeylessProvider(draft));

  const FORMATS: { value: ProviderFormat; label: string }[] = (['chat', 'responses', 'anthropic'] as const)
    .map((value) => ({ value, label: t(`ai.format.${value}`) }));
  const EFFORTS = EFFORT_LEVELS.map((value) => ({ value, label: t(`ai.effort.${value}`) }));

  function applyPreset(preset: ProviderPreset): void {
    presetId = preset.id;
    const fields = fromPreset(preset, taken);
    draft.id = fields.id;
    draft.name = fields.name;
    draft.format = fields.format;
    draft.base_url = fields.base_url;
    keyField?.clear();
    fetchError = '';
    testResult = null;
  }

  function addManualModel(): void {
    if (!modelId.trim()) return;
    const next = addModel(modelId, picked, listed, draft.model);
    manual = [...new Set([...manual, modelId.trim()])];
    picked = next.picked;
    listed = next.listed;
    draft.model = next.star;
    modelId = '';
    query = '';
  }

  function toggleModel(model: string): void {
    picked = picked.includes(model) ? picked.filter((entry) => entry !== model) : [...picked, model];
    draft.model = providerModels(picked, draft.model).model;
  }

  function starModel(model: string): void {
    if (!picked.includes(model)) picked = [...picked, model];
    draft.model = model;
  }

  async function fetchModels(): Promise<void> {
    fetching = true;
    fetchError = '';
    const address = draft.base_url;
    const format = draft.format;
    try {
      const fetched = await aiProviderModels({ ...$state.snapshot(draft), base_url: address.trim() }, key.trim() || null);
      if (address !== draft.base_url || format !== draft.format) return;
      listed = mergeListed([...manual, ...fetched.map((model) => model.id)], picked);
      const contexts = fetched.flatMap(({ id, context }) => context ? [[id, context] as const] : []);
      draft.contexts = { ...draft.contexts, ...Object.fromEntries(contexts) };
      if (!fetched.length) fetchError = t('ai.noModelsListed');
    } catch (failure) {
      fetchError = errorText(failure);
    } finally {
      fetching = false;
    }
  }

  async function testProvider(): Promise<void> {
    testing = true;
    testResult = null;
    try {
      const config = { ...$state.snapshot(draft), ...chosen, name: draft.name.trim() || 'test', base_url: draft.base_url.trim() };
      const reply = await aiProviderTest(config, key.trim() || null);
      testResult = { ok: true, text: t('ai.testOk', { reply: reply.trim() || '…' }) };
    } catch (failure) {
      testResult = { ok: false, text: errorText(failure) };
    } finally {
      testing = false;
    }
  }

  async function save(): Promise<void> {
    if (!canSave || saving) return;
    saving = true;
    error = '';
    const config = {
      ...$state.snapshot(draft),
      ...chosen,
      name: draft.name.trim(),
      base_url: draft.base_url.trim(),
      contexts: Object.fromEntries(Object.entries(draft.contexts).filter(([model]) => picked.includes(model))),
    };
    const existingId = provider?.id ?? persistedId;
    try {
      await updatePrefs((prefs) => {
        const rows = prefs.ai.providers;
        const at = rows.findIndex((row) => row.id === existingId);
        config.id = existingId ?? providerId(config.name, rows.map((row) => row.id));
        if (at >= 0) rows[at] = config;
        else rows.push(config);
      });
      error = settingsError.text;
      if (error) return;
      persistedId = config.id;
      draft.id = config.id;
      if (key.trim()) await aiSetKey(`builtin:${config.id}`, key.trim());
      keyField?.clear();
      onsaved();
    } catch (failure) {
      error = errorText(failure);
    } finally {
      saving = false;
    }
  }
</script>

<Dialog title={t(provider ? 'ai.editProvider' : 'ai.addProvider')} width={560} onclose={() => { if (!saving) onclose(); }}>
  {#if !provider && !persistedId}
    <div class="presets" role="group" aria-label={t('ai.presets')}>
      {#each PRESETS as preset (preset.id)}
        <button type="button" class="secondary" aria-pressed={presetId === preset.id} disabled={saving || fetching || testing} onclick={() => applyPreset(preset)}>{preset.name || t('ai.presetCustom')}</button>
      {/each}
    </div>
  {/if}
  <div class="field-grid">
    <label>{t('ai.providerName')}<input bind:value={draft.name} disabled={saving} /></label>
    <div class="field">
      <span>{t('ai.format')}</span>
      <Select label={t('ai.format')} value={draft.format} options={FORMATS} disabled={saving} onchange={(value) => (draft.format = value)} />
    </div>
  </div>
  <label>
    {t('ai.baseUrl')}
    <input class="mono" bind:value={draft.base_url} spellcheck="false" placeholder="https://…/v1" disabled={saving} />
    <small>{t('ai.baseUrlHint')}</small>
  </label>
  <AiKeyField bind:this={keyField} bind:value={key} label={t('ai.key')} {stored}
    hint={t(stored ? 'ai.keyStored' : keyless ? 'ai.keyOptional' : 'ai.keyHint')}
    reveal={provider ? () => aiRevealKey(`builtin:${provider?.id}`) : undefined}
    disabled={saving} />
  <section class="models">
    <div class="section-header">
      <div><h3>{t('ai.models')}</h3><small>{t('ai.modelsHint')}</small></div>
      <button type="button" class="secondary" disabled={saving || fetching || !canRequest} onclick={() => void fetchModels()}>{t(fetching ? 'ai.fetchingModels' : 'ai.fetchModels')}</button>
    </div>
    <form class="model-entry" onsubmit={(event) => { event.preventDefault(); addManualModel(); }}>
      <input class="mono" bind:value={modelId} placeholder={t('ai.modelId')} aria-label={t('ai.modelId')} spellcheck="false" disabled={saving} />
      <button type="submit" class="secondary" disabled={saving || !modelId.trim()}>{t('ai.add')}</button>
    </form>
    {#if fetchError}<p class="error" role="alert">{fetchError}</p>{/if}
    {#if listed.length > 6}
      <input class="mono" bind:value={query} placeholder={t('ai.searchModels')} aria-label={t('ai.searchModels')} spellcheck="false" />
    {/if}
    {#if shownModels.length}
      <div class="model-list" role="group" aria-label={t('ai.models')}>
        {#each shownModels as model (model)}
          {@const starred = chosen.model === model}
          <div class="model-row">
            <label class="model-pick">
              <input type="checkbox" checked={picked.includes(model)} disabled={saving} onchange={() => toggleModel(model)} />
              <span class="mono" title={model}>{model}</span>
              {#if Object.hasOwn(draft.contexts, model) && draft.contexts[model]}<small class="context">{tokens(draft.contexts[model])}</small>{/if}
            </label>
            <button type="button" class="star" class:starred aria-pressed={starred} title={t('ai.defaultModel')} aria-label={t('ai.makeDefaultModel', { model })} disabled={saving} onclick={() => starModel(model)}>{starred ? '★' : '☆'}</button>
          </div>
        {/each}
      </div>
    {/if}
  </section>
  <section class="advanced">
    <h3>{t('ai.providerDefaults')}</h3>
    <div class="field-grid">
      <label>
        {t('ai.contextWindow')}
        <input class="mono" type="number" min="1000" step="1000" value={draft.context_window ?? ''} placeholder="128000"
          oninput={(event) => (draft.context_window = event.currentTarget.value ? Math.round(event.currentTarget.valueAsNumber) : null)} disabled={saving} />
        <small>{t('ai.contextWindowHint')}</small>
      </label>
      <div class="field">
        <span>{t('ai.effort')}</span>
        <Select label={t('ai.effort')} value={draft.effort} options={EFFORTS} disabled={saving} onchange={(value) => (draft.effort = value)} />
        <small>{t('ai.effortHint')}</small>
      </div>
    </div>
  </section>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#snippet actions()}
    <div class="footer">
      {#if testResult}<p class="result" class:failed={!testResult.ok} role="status">{testResult.text}</p>{/if}
      <div class="footer-buttons">
        <button type="button" class="secondary test" disabled={saving || testing || !canRequest || !chosen.model} onclick={() => void testProvider()}>{t(testing ? 'ai.testing' : 'ai.test')}</button>
        <button type="button" class="secondary" disabled={saving} onclick={onclose}>{t('ai.cancel')}</button>
        <button type="button" class="primary" disabled={saving || !canSave} onclick={() => void save()}>{t('ai.save')}</button>
      </div>
    </div>
  {/snippet}
</Dialog>

<style>
  .presets, .section-header, .model-entry, .model-row, .footer-buttons {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .presets { flex-wrap: wrap; }
  .presets button[aria-pressed='true'] { background: var(--accent-soft); color: var(--accent); }
  .field-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-3); }
  .field { display: flex; flex-direction: column; gap: 5px; color: var(--fg-muted); font-size: var(--text-footnote); font-weight: 500; }
  .mono { font-family: var(--font-mono); }
  h3 { margin: 0; font-size: var(--text-body); font-weight: 600; }
  small { color: var(--fg-muted); }
  .models, .advanced { display: grid; gap: var(--space-2); border-top: 1px solid var(--separator); padding-top: var(--space-3); }
  .section-header { justify-content: space-between; }
  .model-entry input { flex: 1; min-width: 0; }
  .model-list { max-height: 200px; overflow: auto; border: 1px solid var(--separator); border-radius: var(--radius-sm); background: var(--surface); }
  .model-row { padding: var(--space-1) var(--space-2); }
  .model-row + .model-row { border-top: 1px solid var(--separator); }
  .model-row .model-pick { flex: 1; min-width: 0; flex-direction: row; align-items: center; gap: var(--space-2); color: var(--fg); cursor: pointer; }
  .model-pick input { width: auto; margin: 0; }
  .model-pick span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .context { padding: 1px var(--space-1); border-radius: var(--radius-sm); background: var(--fill); }
  .star { padding: 2px var(--space-1); border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--fg-muted); cursor: pointer; }
  .star:hover { background: var(--fill-hover); }
  .star.starred { color: var(--warning); }
  .footer { width: 100%; min-width: 0; }
  .test { margin-right: auto; }
  .result { margin: 0 0 var(--space-2); padding: var(--space-2); border-radius: var(--radius-sm); background: var(--fill); color: var(--success); overflow-wrap: anywhere; }
  .result.failed { color: var(--danger); background: var(--danger-soft); }
  @media (max-width: 520px) {
    .field-grid { grid-template-columns: minmax(0, 1fr); }
    .section-header { flex-wrap: wrap; }
    .footer-buttons { flex-wrap: wrap; justify-content: flex-end; }
    .test { margin-right: 0; }
  }
  @media (pointer: coarse) {
    .model-row { min-height: var(--row-height); }
    .star { min-width: 36px; min-height: 36px; }
  }
</style>
