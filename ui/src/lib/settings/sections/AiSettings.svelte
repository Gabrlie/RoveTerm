<script lang="ts">
  // The AI section: model providers of the built-in assistant, agents on this computer, what a
  // new conversation starts with, and the approval rules.
  import { onMount } from 'svelte';
  import Select from '../../ui/Select.svelte';
  import Switch from '../../ui/Switch.svelte';
  import IconButton from '../../ui/IconButton.svelte';
  import ContextMenu, { type MenuItem } from '../../ui/ContextMenu.svelte';
  import { ask } from '../../ui/confirm.svelte';
  import { errorText, t } from '../../i18n';
  import { aiProviderTest, aiSetKey, type AgentAuth, type AgentChoice, type AiMode, type ProviderFormat, type ProviderPrefs } from '../../ipc';
  import { ai } from '../../ai/ai.svelte';
  import { prefs } from '../prefs.svelte';
  import { editDevice, editPrefs, settingsError } from '../edit.svelte';
  import { notify } from '../../ui/toast.svelte';
  import { customAgentId, fromPreset, PRESETS, type ProviderPreset } from '../providers';

  const rules = $derived(prefs.value.ai);
  const providers = $derived(prefs.value.ai.providers);
  const list = $derived(ai.agents?.agents ?? []);
  const localAgents = $derived(list.filter((entry) => entry.kind === 'acp'));
  const hasKey = (agent: string): boolean => list.some((entry) => entry.agent === agent && entry.has_key);

  // Edited as one entry per line; stored on blur.
  let readOnly = $state(prefs.value.ai.read_only.join('\n'));
  let dangerous = $state(prefs.value.ai.dangerous.join('\n'));

  /** The provider being added or edited, with the key typed for it. */
  let editing = $state<{ original: string | null; draft: ProviderPrefs; key: string; hint: string; keyless: boolean; others: string } | null>(null);
  let testing = $state(false);
  let testResult = $state<{ ok: boolean; text: string } | null>(null);
  let presetMenu = $state<{ x: number; y: number } | null>(null);
  /** The local agent whose key field is open. */
  let keyFor = $state<string | null>(null);
  let agentKey = $state('');
  let adding = $state<{ name: string; command: string; args: string } | null>(null);

  const FORMATS: { value: ProviderFormat; label: string }[] = [
    { value: 'chat', label: t('ai.format.chat') },
    { value: 'responses', label: t('ai.format.responses') },
    { value: 'anthropic', label: t('ai.format.anthropic') },
  ];
  const EFFORTS: { value: 'default' | 'low' | 'medium' | 'high'; label: string }[] = [
    { value: 'default', label: t('ai.effort.default') },
    { value: 'low', label: t('ai.effort.low') },
    { value: 'medium', label: t('ai.effort.medium') },
    { value: 'high', label: t('ai.effort.high') },
  ];
  const MODES: { value: AiMode; label: string }[] = [
    { value: 'ask_all', label: t('ai.mode.ask_all') },
    { value: 'ask_by_risk', label: t('ai.mode.ask_by_risk') },
    { value: 'auto', label: t('ai.mode.auto') },
  ];
  const AUTH: { value: AgentAuth; label: string }[] = [
    { value: 'auto', label: t('ai.auth.auto') },
    { value: 'api_key', label: t('ai.auth.api_key') },
    { value: 'account', label: t('ai.auth.account') },
  ];
  const defaultOptions = $derived([
    { value: '', label: t('ai.defaultAgentFirst') },
    ...list.filter((entry) => entry.enabled).map((entry) => ({ value: entry.agent, label: entry.name })),
  ]);

  function lines(text: string): string[] {
    return text.split('\n').map((line) => line.trim()).filter((line) => line.length > 0);
  }

  async function refresh(): Promise<void> {
    await ai.refreshAgents().catch((error: unknown) => console.warn('AI agents unavailable', error));
  }

  // --- Providers ---

  function presetItems(): MenuItem[] {
    return PRESETS.map((preset) => ({ label: preset.name || t('ai.presetCustom'), onselect: () => startAdding(preset) }));
  }

  function openPresets(event: MouseEvent): void {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    presetMenu = { x: rect.left, y: rect.bottom + 4 };
  }

  function startAdding(preset: ProviderPreset): void {
    const draft = fromPreset(preset, providers.map((provider) => provider.id));
    editing = { original: null, draft, key: '', hint: preset.modelHint, keyless: preset.keyless, others: '' };
    testResult = null;
  }

  function startEditing(provider: ProviderPrefs): void {
    editing = { original: provider.id, draft: structuredClone($state.snapshot(provider)), key: '', hint: '', keyless: false, others: provider.models.join(', ') };
    testResult = null;
  }

  async function saveProvider(): Promise<void> {
    if (!editing) return;
    const { original, draft, key } = editing;
    draft.name = draft.name.trim();
    draft.base_url = draft.base_url.trim();
    draft.model = draft.model.trim();
    draft.models = editing.others
      .split(/[,\n]/)
      .map((model) => model.trim())
      .filter((model) => model.length > 0);
    await editPrefs((prefsDraft) => {
      const at = prefsDraft.ai.providers.findIndex((provider) => provider.id === original);
      if (at >= 0) prefsDraft.ai.providers[at] = draft;
      else prefsDraft.ai.providers.push(draft);
    });
    if (settingsError.text) return;
    if (key.trim()) {
      try {
        await aiSetKey(`builtin:${draft.id}`, key.trim());
      } catch (error) {
        notify(errorText(error), 'error');
        return;
      }
    }
    editing = null;
    await refresh();
  }

  async function removeProvider(provider: ProviderPrefs): Promise<void> {
    const confirmed = await ask({ title: t('ai.removeProvider'), message: t('ai.removeProviderConfirm', { name: provider.name }), confirmLabel: t('ai.remove'), danger: true });
    if (!confirmed) return;
    const agent = `builtin:${provider.id}`;
    await editPrefs((draft) => {
      draft.ai.providers = draft.ai.providers.filter((entry) => entry.id !== provider.id);
      if (draft.ai.default_agent === agent) draft.ai.default_agent = null;
    });
    if (hasKey(agent)) await aiSetKey(agent, null).catch((error: unknown) => notify(errorText(error), 'error'));
    await refresh();
  }

  async function testProvider(): Promise<void> {
    if (!editing) return;
    testing = true;
    testResult = null;
    try {
      const draft = { ...$state.snapshot(editing.draft), name: editing.draft.name.trim() || 'test', model: editing.draft.model.trim() };
      const reply = await aiProviderTest(draft, editing.key.trim() || null);
      testResult = { ok: true, text: t('ai.testOk', { reply: reply.trim() || '…' }) };
    } catch (error) {
      testResult = { ok: false, text: errorText(error) };
    } finally {
      testing = false;
    }
  }

  // --- Local agents ---

  async function setEnabled(agent: AgentChoice, enabled: boolean): Promise<void> {
    const id = agent.agent.slice('acp:'.length);
    await editDevice((draft) => {
      draft.agents.disabled = enabled ? draft.agents.disabled.filter((entry) => entry !== id) : [...draft.agents.disabled, id];
    });
    await refresh();
  }

  async function removeCustom(agent: AgentChoice): Promise<void> {
    const id = agent.agent.slice('acp:'.length);
    await editDevice((draft) => {
      draft.agents.custom = draft.agents.custom.filter((entry) => entry.id !== id);
    });
    if (agent.has_key) await aiSetKey(agent.agent, null).catch(() => undefined);
    if (rules.default_agent === agent.agent) await editPrefs((draft) => (draft.ai.default_agent = null));
    await refresh();
  }

  async function addCustom(): Promise<void> {
    if (!adding || !adding.command.trim()) return;
    const { name, command, args } = adding;
    await editDevice((draft) => {
      draft.agents.custom.push({
        id: customAgentId(name.trim() || command.trim(), draft.agents.custom.map((entry) => entry.id)),
        name: name.trim() || command.trim(),
        command: command.trim(),
        args: args.trim() ? args.trim().split(/\s+/) : [],
      });
    });
    if (settingsError.text) return;
    adding = null;
    await refresh();
  }

  async function saveKey(agent: string, value: string | null): Promise<void> {
    try {
      await aiSetKey(agent, value);
      agentKey = '';
      keyFor = null;
      await refresh();
      notify(value ? t('ai.keySaved') : t('ai.keyRemoved'), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  onMount(() => void refresh());
</script>

<p class="form-note intro">{t('ai.settingsIntro')}</p>

<p class="section-label">{t('ai.providers')}</p>
<div class="form-group">
  {#each providers as provider (provider.id)}
    <div class="form-row item">
      <span class="form-label">{provider.name}<small class="mono">{provider.model}{provider.models.length > 0 ? ` +${provider.models.length}` : ''} · {t(`ai.format.${provider.format}`)}{hasKey(`builtin:${provider.id}`) ? '' : ` · ${t('ai.noKey')}`}</small></span>
      <IconButton icon="edit" small label={t('ai.edit')} onclick={() => startEditing(provider)} />
      <IconButton icon="remove" small label={t('ai.remove')} onclick={() => void removeProvider(provider)} />
    </div>
  {/each}
  {#if providers.length === 0 && !editing}
    <div class="form-row"><span class="form-label"><small>{t('ai.noProviders')}</small></span></div>
  {/if}
  {#if editing}
    <div class="editor">
      <div class="form-row">
        <span class="form-label">{t('ai.providerName')}</span>
        <input class="field" bind:value={editing.draft.name} aria-label={t('ai.providerName')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('ai.format')}<small>{t('ai.formatHint')}</small></span>
        <div class="control"><Select label={t('ai.format')} value={editing.draft.format} options={FORMATS} onchange={(value) => editing && (editing.draft.format = value)} /></div>
      </div>
      <div class="form-row">
        <span class="form-label">{t('ai.baseUrl')}<small>{t('ai.baseUrlHint')}</small></span>
        <input class="field mono" bind:value={editing.draft.base_url} spellcheck="false" placeholder="https://…/v1" aria-label={t('ai.baseUrl')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('ai.model')}</span>
        <input class="field mono" bind:value={editing.draft.model} spellcheck="false" placeholder={editing.hint ? t('ai.modelExample', { model: editing.hint }) : ''} aria-label={t('ai.model')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('ai.otherModels')}<small>{t('ai.otherModelsHint')}</small></span>
        <input class="field mono" bind:value={editing.others} spellcheck="false" aria-label={t('ai.otherModels')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('ai.effort')}<small>{t('ai.effortHint')}</small></span>
        <div class="control"><Select label={t('ai.effort')} value={editing.draft.effort ?? 'default'} options={EFFORTS} onchange={(value) => editing && (editing.draft.effort = value === 'default' ? null : value)} /></div>
      </div>
      <div class="form-row">
        <span class="form-label">{t('ai.key')}<small>{editing.keyless ? t('ai.keyOptional') : editing.original && hasKey(`builtin:${editing.original}`) ? t('ai.keyStored') : t('ai.keyHint')}</small></span>
        <input class="field" type="password" bind:value={editing.key} autocomplete="off" placeholder={editing.original && hasKey(`builtin:${editing.original}`) ? '••••••••' : 'sk-…'} aria-label={t('ai.key')} />
      </div>
      {#if testResult}<p class="form-note result" class:failed={!testResult.ok}>{testResult.text}</p>{/if}
      <div class="form-row buttons">
        <button type="button" class="secondary" disabled={testing || !editing.draft.base_url.trim() || !editing.draft.model.trim()} onclick={() => void testProvider()}>{testing ? t('ai.testing') : t('ai.test')}</button>
        <span class="spacer"></span>
        <button type="button" class="secondary" onclick={() => (editing = null)}>{t('ai.cancel')}</button>
        <button type="button" class="primary" disabled={!editing.draft.name.trim() || !editing.draft.base_url.trim() || !editing.draft.model.trim()} onclick={() => void saveProvider()}>{t('ai.save')}</button>
      </div>
    </div>
  {:else}
    <div class="form-row">
      <button type="button" class="plain add" onclick={openPresets}>{t('ai.addProvider')}</button>
    </div>
  {/if}
</div>

<p class="section-label">{t('ai.localAgents')}</p>
<div class="form-group">
  {#if !ai.agents}
    <div class="form-row"><span class="form-label"><small>…</small></span></div>
  {:else if localAgents.length === 0}
    <div class="form-row"><span class="form-label"><small>{t('ai.noLocalAgents')}</small></span></div>
  {/if}
  {#each localAgents as agent (agent.agent)}
    <div class="form-row item" class:off={!agent.enabled}>
      <span class="form-label">{agent.name}<small class="mono">{agent.via_npx ? t('ai.viaNpx') : agent.detail}{agent.has_key ? ` · ${t('ai.keyStoredShort')}` : ''}</small></span>
      <IconButton icon="key" small label={t('ai.key')} onclick={() => ((keyFor = keyFor === agent.agent ? null : agent.agent), (agentKey = ''))} />
      {#if agent.custom}
        <IconButton icon="remove" small label={t('ai.remove')} onclick={() => void removeCustom(agent)} />
      {:else}
        <Switch label={agent.name} checked={agent.enabled} onchange={(checked) => void setEnabled(agent, checked)} />
      {/if}
    </div>
    {#if keyFor === agent.agent}
      <div class="form-row sub">
        <span class="form-label"><small>{agent.has_key ? t('ai.keyStored') : t('ai.agentKeyHint')}</small></span>
        <input class="field" type="password" bind:value={agentKey} autocomplete="off" placeholder={agent.has_key ? '••••••••' : 'sk-…'} aria-label={t('ai.key')} />
        <button type="button" class="secondary" disabled={!agentKey.trim()} onclick={() => void saveKey(agent.agent, agentKey.trim())}>{t('ai.keySave')}</button>
        {#if agent.has_key}<button type="button" class="plain danger-text" onclick={() => void saveKey(agent.agent, null)}>{t('ai.keyRemove')}</button>{/if}
      </div>
    {/if}
  {/each}
  {#if adding}
    <div class="editor">
      <div class="form-row">
        <span class="form-label">{t('ai.agentName')}</span>
        <input class="field" bind:value={adding.name} aria-label={t('ai.agentName')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('ai.command')}<small>{t('ai.commandHint')}</small></span>
        <input class="field mono" bind:value={adding.command} spellcheck="false" placeholder="my-agent-acp" aria-label={t('ai.command')} />
      </div>
      <div class="form-row">
        <span class="form-label">{t('ai.args')}</span>
        <input class="field mono" bind:value={adding.args} spellcheck="false" placeholder="--acp" aria-label={t('ai.args')} />
      </div>
      <div class="form-row buttons">
        <span class="spacer"></span>
        <button type="button" class="secondary" onclick={() => (adding = null)}>{t('ai.cancel')}</button>
        <button type="button" class="primary" disabled={!adding.command.trim()} onclick={() => void addCustom()}>{t('ai.add')}</button>
      </div>
    </div>
  {:else}
    <div class="form-row">
      <button type="button" class="plain add" onclick={() => (adding = { name: '', command: '', args: '' })}>{t('ai.addAgent')}</button>
    </div>
  {/if}
  <div class="form-row">
    <span class="form-label">{t('ai.auth')}<small>{t('ai.authHint')}</small></span>
    <div class="control"><Select label={t('ai.auth')} value={prefs.device.agents.auth} options={AUTH} onchange={(value) => void editDevice((draft) => (draft.agents.auth = value))} /></div>
  </div>
</div>

<p class="section-label">{t('ai.defaults')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('ai.defaultAgent')}</span>
    <div class="control"><Select label={t('ai.defaultAgent')} value={rules.default_agent ?? ''} options={defaultOptions} onchange={(value) => void editPrefs((draft) => (draft.ai.default_agent = value || null))} /></div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('ai.defaultMode')}<small>{t(`ai.modeHint.${rules.mode}`)}</small></span>
    <div class="control"><Select label={t('ai.defaultMode')} value={rules.mode} options={MODES} onchange={(value) => void editPrefs((draft) => (draft.ai.mode = value))} /></div>
  </div>
</div>

<p class="section-label">{t('ai.approvals')}</p>
<div class="form-group">
  <div class="form-row stacked">
    <span class="form-label">{t('ai.readOnlyRules')}<small>{t('ai.readOnlyHint')}</small></span>
    <textarea class="mono" rows="3" bind:value={readOnly} spellcheck="false" onblur={() => void editPrefs((draft) => (draft.ai.read_only = lines(readOnly)))} aria-label={t('ai.readOnlyRules')}></textarea>
  </div>
  <div class="form-row stacked">
    <span class="form-label">{t('ai.dangerousRules')}<small>{t('ai.dangerousHint')}</small></span>
    <textarea class="mono" rows="3" bind:value={dangerous} spellcheck="false" onblur={() => void editPrefs((draft) => (draft.ai.dangerous = lines(dangerous)))} aria-label={t('ai.dangerousRules')}></textarea>
  </div>
</div>
{#if settingsError.text}<p class="form-note error">{settingsError.text}</p>{/if}

{#if presetMenu}
  <ContextMenu x={presetMenu.x} y={presetMenu.y} items={presetItems()} onclose={() => (presetMenu = null)} />
{/if}

<style>
  .intro {
    margin: 0 0 var(--space-2);
    font-size: var(--text-footnote);
  }
  .item .form-label small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item.off .form-label {
    opacity: 0.55;
  }
  .sub {
    padding-left: var(--space-5);
  }
  .editor {
    margin: var(--space-1) 0;
    border-radius: var(--radius-sm);
    background: var(--fill);
  }
  .buttons {
    gap: var(--space-2);
  }
  .spacer {
    flex: 1;
  }
  .add {
    color: var(--accent);
  }
  .result {
    margin: 0;
    padding: 0 var(--space-3);
    color: var(--success);
    overflow-wrap: anywhere;
  }
  .result.failed,
  .error {
    color: var(--danger);
  }
  .field {
    width: 240px;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
  }
  .control {
    width: 200px;
  }
  .stacked {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
  }
  textarea {
    resize: vertical;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--separator);
    color: var(--fg);
  }
  .danger-text {
    color: var(--danger);
  }
</style>
