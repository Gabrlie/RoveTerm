<script lang="ts">
  import { onMount } from 'svelte';
  import Dialog from '../../ui/Dialog.svelte';
  import Select from '../../ui/Select.svelte';
  import Switch from '../../ui/Switch.svelte';
  import IconButton from '../../ui/IconButton.svelte';
  import TagInput from '../../ui/TagInput.svelte';
  import { ask } from '../../ui/confirm.svelte';
  import { writeClipboard } from '../../ui/clipboard';
  import { notify } from '../../ui/toast.svelte';
  import { errorText, t } from '../../i18n';
  import {
    aiRevealKey, aiRevealReviewKey, aiSetKey, aiSetReviewKey, externalInstall, externalSet, externalStatus,
    type AgentAuth, type AgentChoice, type AiMode, type CustomAgent, type ExternalStatus, type Preferences, type ProviderPrefs,
  } from '../../ipc';
  import { ai } from '../../ai/ai.svelte';
  import AgentLoginDialog from '../../ai/AgentLoginDialog.svelte';
  import { isMobile } from '../../platform';
  import { prefs } from '../prefs.svelte';
  import { editDevice, editPrefs, settingsError } from '../edit.svelte';
  import { isKeylessProvider } from '../providers';
  import AiKeyField from './AiKeyField.svelte';
  import CustomAgentDialog from './CustomAgentDialog.svelte';
  import ProviderEditorDialog from './ProviderEditorDialog.svelte';

  const TABS = ['models', 'safety', 'external'] as const;
  let tab = $state<(typeof TABS)[number]>('models');
  let providerEditor = $state<{ provider: ProviderPrefs | null } | null>(null);
  let customEditor = $state<{ agent: CustomAgent | null } | null>(null);
  let keyFor = $state<string | null>(null);
  let loginAgent = $state<AgentChoice | null>(null);
  let key = $state('');
  let keySaving = $state(false);
  let jevKey = $state('');
  let jevField: AiKeyField | undefined = $state();
  let jevSaving = $state(false);

  const rules = $derived(prefs.value.ai);
  const providers = $derived(rules.providers);
  const list = $derived(ai.agents?.agents ?? []);
  const localAgents = $derived(list.filter((entry) => entry.kind === 'acp'));
  const keyAgent = $derived(localAgents.find((entry) => entry.agent === keyFor));
  const hasKey = (agent: string): boolean => list.some((entry) => entry.agent === agent && entry.has_key);

  let pendingPrefs = Promise.resolve();
  function updatePrefs(change: (draft: Preferences) => void): Promise<void> {
    pendingPrefs = pendingPrefs.then(() => editPrefs(change));
    return pendingPrefs;
  }

  type RuleList = 'read_only' | 'dangerous';
  function updateRules(field: RuleList, change: (rules: readonly string[]) => string[]): Promise<void> {
    return updatePrefs((draft) => { draft.ai[field] = change(draft.ai[field]); });
  }

  const REVIEWERS: { value: 'model' | 'chosen' | 'jev'; label: string }[] = [
    { value: 'model', label: t('ai.reviewer.model') },
    { value: 'chosen', label: t('ai.reviewer.chosen') },
    { value: 'jev', label: t('ai.reviewer.jev') },
  ];
  /** Every model of the providers, as `provider id` + `model`, for the chosen judge. */
  const JUDGE_MODELS = $derived(
    providers.flatMap((provider) => [provider.model, ...provider.models].map((model) => ({ value: `${provider.id}\n${model}`, label: `${provider.name} · ${model}` }))),
  );

  /** Picks the judge kind; the chosen one starts on the first model offered. */
  function setReviewer(value: 'model' | 'chosen' | 'jev'): void {
    void updatePrefs((draft) => {
      draft.ai.reviewer = value;
      if (value === 'chosen' && !draft.ai.review_model) {
        const first = draft.ai.providers[0];
        draft.ai.review_model = first ? { provider: first.id, model: first.model } : null;
      }
    });
  }
  const CONFIDENCES = [60, 70, 80, 90, 95];
  const STEPS = [10, 20, 40, 80, 120, 200];
  const TIMEOUTS = [60, 120, 180, 300, 600, 1200, 1800];
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

  async function refresh(): Promise<void> {
    try {
      await ai.refreshAgents();
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  function providerSaved(): void {
    providerEditor = null;
    void refresh();
  }

  function customSaved(): void {
    customEditor = null;
    void refresh();
  }

  function startEditingCustom(agent: AgentChoice): void {
    const config = prefs.device.agents.custom.find((entry) => `acp:${entry.id}` === agent.agent);
    if (config) customEditor = { agent: config };
  }

  function closeKey(): void {
    key = '';
    keyFor = null;
  }

  async function saveAgentKey(agent: string, value: string | null): Promise<void> {
    keySaving = true;
    try {
      await aiSetKey(agent, value);
      closeKey();
      await refresh();
      notify(t(value ? 'ai.keySaved' : 'ai.keyRemoved'), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    } finally {
      keySaving = false;
    }
  }

  async function saveJevKey(value: string | null): Promise<void> {
    jevSaving = true;
    try {
      await aiSetReviewKey(value);
      jevField?.clear();
      await refresh();
    } catch (error) {
      notify(errorText(error), 'error');
    } finally {
      jevSaving = false;
    }
  }

  async function removeProvider(provider: ProviderPrefs): Promise<void> {
    const confirmed = await ask({ title: t('ai.removeProvider'), message: t('ai.removeProviderConfirm', { name: provider.name }), confirmLabel: t('ai.remove'), danger: true });
    if (!confirmed) return;
    const agent = `builtin:${provider.id}`;
    await updatePrefs((draft) => {
      draft.ai.providers = draft.ai.providers.filter((entry) => entry.id !== provider.id);
      if (draft.ai.default_agent === agent) draft.ai.default_agent = null;
    });
    if (settingsError.text) return;
    if (hasKey(agent)) await aiSetKey(agent, null).catch((error: unknown) => notify(errorText(error), 'error'));
    await refresh();
  }

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
    if (settingsError.text) return;
    if (agent.has_key) await aiSetKey(agent.agent, null).catch((error: unknown) => notify(errorText(error), 'error'));
    if (rules.default_agent === agent.agent) await updatePrefs((draft) => (draft.ai.default_agent = null));
    await refresh();
  }

  /** Access for agents the user runs on their own. */
  let external = $state<ExternalStatus | null>(null);
  /** What the last one-click setup printed, or why it failed. */
  let installNote = $state<{ ok: boolean; text: string } | null>(null);
  let installing = $state(false);

  async function setExternal(enabled: boolean): Promise<void> {
    try {
      external = await externalSet(enabled);
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function install(client: 'claude' | 'codex'): Promise<void> {
    installing = true;
    installNote = null;
    try {
      const output = await externalInstall(client);
      installNote = { ok: true, text: t('ai.externalInstalled', { agent: t(`ai.externalClient.${client}`) }) + (output ? `\n${output}` : '') };
    } catch (error) {
      installNote = { ok: false, text: errorText(error) };
    } finally {
      installing = false;
    }
  }

  /** The command, quoted for a shell where a part has spaces. */
  const externalCommand = $derived((external?.command ?? []).map((part) => (/\s/.test(part) ? `"${part}"` : part)).join(' '));

  onMount(() => {
    void refresh();
    externalStatus()
      .then((status) => (external = status))
      .catch((error: unknown) => notify(errorText(error), 'error'));
  });
</script>

<div class="segmented ai-tabs" role="group" aria-label={t('ai.settingsTabs')}>
  {#each TABS as item}
    <button type="button" aria-pressed={tab === item} onclick={() => (tab = item)}>{t(`ai.tab.${item}`)}</button>
  {/each}
</div>

{#if tab === 'models'}
  <div class="section-heading">
    <h3 class="section-label">{t('ai.providers')}</h3>
    <button type="button" class="primary" onclick={() => (providerEditor = { provider: null })}>+ {t('ai.addProvider')}</button>
  </div>
  <div class="form-group">
    {#each providers as provider (provider.id)}
      {@const stored = hasKey(`builtin:${provider.id}`)}
      <div class="form-row item">
        <div class="form-label">
          <div class="item-name">{provider.name}<span class="badge">{t(`ai.format.${provider.format}`)}</span></div>
          <small class="mono model-summary">
            <span title={provider.model}>★ {provider.model}</span>
            {#if provider.models.length}<span>+{provider.models.length}</span>{/if}
          </small>
        </div>
        <span class="badge" class:stored>{t(stored ? 'ai.keyStoredShort' : isKeylessProvider(provider) ? 'ai.keyless' : 'ai.noKey')}</span>
        <div class="row-actions">
          <IconButton icon="edit" small label={t('ai.edit')} onclick={() => (providerEditor = { provider })} />
          <IconButton icon="remove" small danger label={t('ai.remove')} onclick={() => void removeProvider(provider)} />
        </div>
      </div>
    {:else}
      <div class="form-row"><span class="form-label"><small>{t('ai.noProviders')}</small></span></div>
    {/each}
  </div>

  <div class="section-heading">
    <h3 class="section-label">{t('ai.localAgents')}</h3>
    <button type="button" class="primary" onclick={() => (customEditor = { agent: null })}>+ {t('ai.addAgent')}</button>
  </div>
  <div class="form-group">
    {#each localAgents as agent (agent.agent)}
      <div class="form-row item">
        <span class="form-label" class:off={!agent.enabled}>
          {agent.name}
          <small class="mono agent-command" title={agent.detail}>{agent.via_npx ? t('ai.viaNpx') : agent.detail}</small>
        </span>
        <span class="badge" class:stored={agent.has_key}>{t(agent.has_key ? 'ai.keyStoredShort' : 'ai.noKey')}</span>
        <div class="row-actions">
          {#if !isMobile}
            <IconButton icon="logIn" small label={t('ai.login.action')} onclick={() => (loginAgent = agent)} />
          {/if}
          <IconButton icon="key" small label={t('ai.key')} disabled={keySaving} onclick={() => (keyFor = agent.agent)} />
          {#if agent.custom}
            <IconButton icon="edit" small label={t('ai.edit')} onclick={() => startEditingCustom(agent)} />
            <IconButton icon="remove" small danger label={t('ai.remove')} onclick={() => void removeCustom(agent)} />
          {:else}
            <Switch label={agent.name} checked={agent.enabled} onchange={(checked) => void setEnabled(agent, checked)} />
          {/if}
        </div>
      </div>
    {:else}
      <div class="form-row"><span class="form-label"><small>{ai.agents ? t('ai.noLocalAgents') : t('ai.loadingAgents')}</small></span></div>
    {/each}
    <div class="form-row">
      <span class="form-label">{t('ai.auth')}<small>{t('ai.authHint')}</small></span>
      <div class="control"><Select label={t('ai.auth')} value={prefs.device.agents.auth} options={AUTH} onchange={(value) => void editDevice((draft) => (draft.agents.auth = value))} /></div>
    </div>
  </div>
{:else if tab === 'safety'}
  <p class="section-label">{t('ai.defaults')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{t('ai.defaultAgent')}</span>
      <div class="control"><Select label={t('ai.defaultAgent')} value={rules.default_agent ?? ''} options={defaultOptions} onchange={(value) => void updatePrefs((draft) => (draft.ai.default_agent = value || null))} /></div>
    </div>
    <div class="form-row">
      <span class="form-label">{t('ai.defaultMode')}<small>{t(`ai.modeHint.${rules.mode}`)}</small></span>
      <div class="control"><Select label={t('ai.defaultMode')} value={rules.mode} options={MODES} onchange={(value) => void updatePrefs((draft) => (draft.ai.mode = value))} /></div>
    </div>
    <div class="form-row">
      <span class="form-label">{t('ai.maxSteps')}<small>{t('ai.maxStepsHint')}</small></span>
      <div class="control"><Select label={t('ai.maxSteps')} value={String(rules.max_steps)} options={[...new Set([...STEPS, rules.max_steps])].sort((a, b) => a - b).map((n) => ({ value: String(n), label: String(n) }))} onchange={(value) => void updatePrefs((draft) => (draft.ai.max_steps = Number(value)))} /></div>
    </div>
    <div class="form-row">
      <span class="form-label">{t('ai.requestTimeout')}<small>{t('ai.requestTimeoutHint')}</small></span>
      <div class="control"><Select label={t('ai.requestTimeout')} value={String(rules.request_timeout_secs)} options={[...new Set([...TIMEOUTS, rules.request_timeout_secs])].sort((a, b) => a - b).map((n) => ({ value: String(n), label: t('ai.seconds', { n }) }))} onchange={(value) => void updatePrefs((draft) => (draft.ai.request_timeout_secs = Number(value)))} /></div>
    </div>
  </div>

  <p class="section-label">{t('ai.approvals')}</p>
  <div class="form-group">
    <div class="form-row stacked">
      <span class="form-label">{t('ai.readOnlyRules')}<small>{t('ai.readOnlyHint')}</small></span>
      <TagInput label={t('ai.readOnlyRules')} rules={rules.read_only} onchange={(change) => updateRules('read_only', change)} />
    </div>
    <div class="form-row stacked">
      <span class="form-label">{t('ai.dangerousRules')}<small>{t('ai.dangerousHint')}</small></span>
      <TagInput label={t('ai.dangerousRules')} rules={rules.dangerous} onchange={(change) => updateRules('dangerous', change)} />
    </div>
  </div>

  <p class="section-label">{t('ai.review')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{t('ai.reviewerLabel')}<small>{t('ai.reviewerHint')}</small></span>
      <div class="control"><Select label={t('ai.reviewerLabel')} value={rules.reviewer} options={REVIEWERS} onchange={setReviewer} /></div>
    </div>
    {#if rules.reviewer === 'chosen'}
      <div class="form-row">
        <span class="form-label">{t('ai.reviewModel')}<small>{t('ai.reviewModelHint')}</small></span>
        <div class="control"><Select label={t('ai.reviewModel')} value={rules.review_model ? `${rules.review_model.provider}\n${rules.review_model.model}` : ''} options={JUDGE_MODELS} onchange={(value) => void updatePrefs((draft) => { const [provider = '', model = ''] = value.split('\n'); draft.ai.review_model = { provider, model }; })} /></div>
      </div>
    {/if}
    <div class="form-row">
      <span class="form-label">{t('ai.reviewConfidence')}<small>{t('ai.reviewConfidenceHint')}</small></span>
      <div class="control"><Select label={t('ai.reviewConfidence')} value={String(rules.review_confidence)} options={[...new Set([...CONFIDENCES, rules.review_confidence])].sort((a, b) => a - b).map((n) => ({ value: String(n), label: `${n}%` }))} onchange={(value) => void updatePrefs((draft) => (draft.ai.review_confidence = Number(value)))} /></div>
    </div>
    {#if rules.reviewer === 'jev'}
      <div class="form-row stacked">
        <AiKeyField bind:this={jevField} bind:value={jevKey} label={t('ai.jevKey')}
          hint={t(ai.agents?.jev_key ? 'ai.keyStored' : 'ai.jevKeyHint')}
          stored={ai.agents?.jev_key ?? false} reveal={aiRevealReviewKey} disabled={jevSaving} />
        <div class="row-actions">
          <button type="button" class="secondary" disabled={jevSaving || !jevKey.trim()} onclick={() => void saveJevKey(jevKey.trim())}>{t('ai.keySave')}</button>
          {#if ai.agents?.jev_key}<button type="button" class="plain danger-text" disabled={jevSaving} onclick={() => void saveJevKey(null)}>{t('ai.keyRemove')}</button>{/if}
        </div>
      </div>
    {/if}
  </div>
{:else}
  <p class="section-label">{t('ai.externalAccess')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{t('ai.externalEnable')}<small>{t('ai.externalEnableHint')}</small></span>
      <Switch label={t('ai.externalEnable')} checked={external?.enabled ?? false} onchange={(checked) => void setExternal(checked)} />
    </div>
    {#if external?.enabled}
      <div class="form-row">
        <span class="form-label">{t('ai.externalSetup')}<small>{t('ai.externalSetupHint')}</small></span>
        <button type="button" class="secondary" disabled={installing} onclick={() => void install('claude')}>{t('ai.externalClient.claude')}</button>
        <button type="button" class="secondary" disabled={installing} onclick={() => void install('codex')}>{t('ai.externalClient.codex')}</button>
      </div>
      <div class="form-row stacked">
        <span class="form-label">{t('ai.externalCommand')}<small>{t('ai.externalCommandHint')}</small></span>
        <div class="command-line">
          <code class="mono selectable">{externalCommand}</code>
          <IconButton icon="copy" small label={t('edit.copy')} onclick={() => void writeClipboard(externalCommand)} />
        </div>
      </div>
    {/if}
  </div>
  {#if installNote}<p class="install-note" class:failed={!installNote.ok} role="status">{installNote.text}</p>{/if}
{/if}

{#if providerEditor}
  <ProviderEditorDialog provider={providerEditor.provider} taken={providers.map((provider) => provider.id)}
    stored={providerEditor.provider ? hasKey(`builtin:${providerEditor.provider.id}`) : false}
    {updatePrefs} onclose={() => (providerEditor = null)} onsaved={providerSaved} />
{/if}
{#if customEditor}
  <CustomAgentDialog agent={customEditor.agent} onclose={() => (customEditor = null)} onsaved={customSaved} />
{/if}
{#if loginAgent}
  {@const target = loginAgent}
  <AgentLoginDialog
    agent={target.agent}
    agentName={target.name}
    onclose={() => (loginAgent = null)}
    onsignedin={() => void refresh()}
    onapikey={() => {
      loginAgent = null;
      keyFor = target.agent;
    }}
  />
{/if}
{#if keyAgent}
  {@const agent = keyAgent.agent}
  <Dialog title={t('ai.agentKeyTitle', { name: keyAgent.name })} width={460} onclose={() => { if (!keySaving) closeKey(); }}>
    <AiKeyField bind:value={key} label={t('ai.key')} hint={t(keyAgent.has_key ? 'ai.keyStored' : 'ai.agentKeyHint')}
      stored={keyAgent.has_key} reveal={() => aiRevealKey(agent)} disabled={keySaving} />
    {#snippet actions()}
      {#if keyAgent.has_key}<button type="button" class="plain danger-text remove-key" disabled={keySaving} onclick={() => void saveAgentKey(agent, null)}>{t('ai.keyRemove')}</button>{/if}
      <button type="button" class="secondary" disabled={keySaving} onclick={closeKey}>{t('ai.cancel')}</button>
      <button type="button" class="primary" disabled={keySaving || !key.trim()} onclick={() => void saveAgentKey(agent, key.trim())}>{t('ai.keySave')}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .ai-tabs { display: flex; }
  .ai-tabs > button { flex: 1; min-width: 0; height: auto; min-height: var(--control-height); padding: var(--space-1); }
  .section-heading { display: flex; justify-content: space-between; align-items: center; gap: var(--space-2); }
  .section-heading button { flex: none; }
  .item { gap: var(--space-2); }
  .item-name { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-1) var(--space-2); overflow-wrap: anywhere; }
  .item .off { opacity: 0.65; }
  .model-summary { display: flex; gap: var(--space-2); }
  .model-summary span:first-child, .agent-command { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge { flex: none; padding: 2px var(--space-2); border-radius: var(--radius-sm); background: var(--fill); color: var(--fg-muted); font-size: var(--text-caption); }
  .badge.stored { background: var(--accent-soft); color: var(--accent); }
  .row-actions { display: flex; gap: var(--space-1); align-items: center; flex: none; }
  .control { width: min(200px, 45%); }
  .stacked { flex-direction: column; align-items: stretch; gap: var(--space-2); }
  .mono { font-family: var(--font-mono); }
  .command-line { display: flex; align-items: flex-start; gap: var(--space-2); }
  .command-line code {
    flex: 1;
    min-width: 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--separator);
    font-size: var(--text-footnote);
    overflow-wrap: anywhere;
  }
  .install-note { margin: 0; padding: var(--space-2) var(--space-3); border-radius: var(--radius-sm); background: var(--fill); color: var(--success); white-space: pre-wrap; overflow-wrap: anywhere; }
  .install-note.failed { background: var(--danger-soft); }
  .failed, .danger-text { color: var(--danger); }
  .remove-key { margin-right: auto; }
</style>
