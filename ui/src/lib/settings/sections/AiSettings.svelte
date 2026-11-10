<script lang="ts">
  // The agent program, how it signs in, and how much RoveTerm asks before it acts.
  import { onMount } from 'svelte';
  import Select from '../../ui/Select.svelte';
  import { errorText, t } from '../../i18n';
  import { aiSetKey, type AgentAuth, type AiMode } from '../../ipc';
  import { ai } from '../../ai/ai.svelte';
  import { prefs } from '../prefs.svelte';
  import { editDevice, editPrefs, settingsError } from '../edit.svelte';
  import { notify } from '../../ui/toast.svelte';

  const agent = $derived(prefs.device.agents);
  const rules = $derived(prefs.value.ai);
  let command = $state(prefs.device.agents.custom[0]?.command ?? '');
  let args = $state((prefs.device.agents.custom[0]?.args ?? []).join(' '));
  let key = $state('');
  // Edited as one entry per line; stored on blur.
  let readOnly = $state(prefs.value.ai.read_only.join('\n'));
  let dangerous = $state(prefs.value.ai.dangerous.join('\n'));

  const MODES: { value: AiMode; label: string }[] = [
    { value: 'ask_by_risk', label: t('ai.mode.ask_by_risk') },
    { value: 'ask_all', label: t('ai.mode.ask_all') },
    { value: 'auto', label: t('ai.mode.auto') },
  ];
  const AUTH: { value: AgentAuth; label: string }[] = [
    { value: 'auto', label: t('ai.auth.auto') },
    { value: 'api_key', label: t('ai.auth.api_key') },
    { value: 'account', label: t('ai.auth.account') },
  ];

  function lines(text: string): string[] {
    return text.split('\n').map((line) => line.trim()).filter((line) => line.length > 0);
  }

  async function saveAgent(): Promise<void> {
    await editDevice((draft) => {
      const rest = draft.agents.custom.slice(1);
      draft.agents.custom = command.trim() ? [{ id: 'custom-1', name: 'Agent', command: command.trim(), args: args.trim() ? args.trim().split(/\s+/) : [] }, ...rest] : rest;
    });
    await ai.refreshStatus().catch(() => undefined);
  }

  async function saveKey(value: string | null): Promise<void> {
    try {
      await aiSetKey(value);
      key = '';
      await ai.refreshStatus();
      notify(value ? t('ai.keySaved') : t('ai.keyRemoved'), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  onMount(() => {
    ai.refreshStatus().catch((error: unknown) => console.warn('AI status unavailable', error));
  });
</script>

<p class="form-note intro">{t('ai.settingsIntro')}</p>

<p class="section-label">{t('ai.agent')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('ai.detected')}
      <small class="path" title={ai.status?.agent ? [ai.status.agent.program, ...ai.status.agent.args].join(' ') : ''}>
        {#if !ai.status}…{:else if ai.status.agent}{t(`ai.source.${ai.status.agent.source}`)} · {[ai.status.agent.program, ...ai.status.agent.args].join(' ')}{:else}{t('ai.noAgentShort')}{/if}
      </small>
    </span>
  </div>
  <div class="form-row">
    <span class="form-label">{t('ai.command')}<small>{t('ai.commandHint')}</small></span>
    <input class="field mono" bind:value={command} spellcheck="false" placeholder="codex-acp" onblur={() => void saveAgent()} aria-label={t('ai.command')} />
  </div>
  <div class="form-row">
    <span class="form-label">{t('ai.args')}</span>
    <input class="field mono" bind:value={args} spellcheck="false" onblur={() => void saveAgent()} aria-label={t('ai.args')} />
  </div>
</div>

<p class="section-label">{t('ai.signIn')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('ai.auth')}<small>{t('ai.authHint')}</small></span>
    <div class="control"><Select label={t('ai.auth')} value={agent.auth} options={AUTH} onchange={(value) => void editDevice((draft) => (draft.agents.auth = value))} /></div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('ai.key')}<small>{ai.status?.has_key ? t('ai.keyStored') : t('ai.keyHint')}</small></span>
    <input class="field" type="password" bind:value={key} autocomplete="off" placeholder={ai.status?.has_key ? '••••••••' : 'sk-…'} aria-label={t('ai.key')} />
    <button type="button" class="secondary" disabled={!key.trim()} onclick={() => void saveKey(key)}>{t('ai.keySave')}</button>
    {#if ai.status?.has_key}
      <button type="button" class="plain danger-text" onclick={() => void saveKey(null)}>{t('ai.keyRemove')}</button>
    {/if}
  </div>
</div>

<p class="section-label">{t('ai.approvals')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('ai.defaultMode')}<small>{t(`ai.modeHint.${rules.mode}`)}</small></span>
    <div class="control"><Select label={t('ai.defaultMode')} value={rules.mode} options={MODES} onchange={(value) => void editPrefs((draft) => (draft.ai.mode = value))} /></div>
  </div>
  <div class="form-row stacked">
    <span class="form-label">{t('ai.readOnlyRules')}<small>{t('ai.readOnlyHint')}</small></span>
    <textarea class="mono" rows="3" bind:value={readOnly} spellcheck="false" onblur={() => void editPrefs((draft) => (draft.ai.read_only = lines(readOnly)))} aria-label={t('ai.readOnlyRules')}></textarea>
  </div>
  <div class="form-row stacked">
    <span class="form-label">{t('ai.dangerousRules')}<small>{t('ai.dangerousHint')}</small></span>
    <textarea class="mono" rows="3" bind:value={dangerous} spellcheck="false" onblur={() => void editPrefs((draft) => (draft.ai.dangerous = lines(dangerous)))} aria-label={t('ai.dangerousRules')}></textarea>
  </div>
</div>
{#if settingsError.text}<p class="form-note">{settingsError.text}</p>{/if}

<style>
  .intro {
    margin: 0 0 var(--space-2);
    font-size: var(--text-footnote);
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .field {
    width: 220px;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
  }
  .control {
    width: 180px;
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
