<script lang="ts">
  import Dialog from '../../ui/Dialog.svelte';
  import { t } from '../../i18n';
  import type { CustomAgent } from '../../ipc';
  import { customAgentId } from '../providers';
  import { editDevice, settingsError } from '../edit.svelte';

  interface Props {
    agent: CustomAgent | null;
    onclose: () => void;
    onsaved: () => void;
  }

  let { agent, onclose, onsaved }: Props = $props();
  function initialFields() {
    return { name: agent?.name ?? '', command: agent?.command ?? '', args: agent?.args.join('\n') ?? '' };
  }

  let fields = $state(initialFields());
  let saving = $state(false);
  let error = $state('');

  async function save(): Promise<void> {
    const command = fields.command.trim();
    if (!command || saving) return;
    saving = true;
    error = '';
    const name = fields.name.trim() || command;
    const args = fields.args === agent?.args.join('\n') ? [...agent.args] : fields.args ? fields.args.split('\n') : [];
    await editDevice((draft) => {
      if (agent) {
        const at = draft.agents.custom.findIndex((entry) => entry.id === agent?.id);
        if (at >= 0) draft.agents.custom[at] = { id: agent.id, name, command, args };
      } else {
        const id = customAgentId(name, draft.agents.custom.map((entry) => entry.id));
        draft.agents.custom.push({ id, name, command, args });
      }
    });
    saving = false;
    error = settingsError.text;
    if (!error) onsaved();
  }
</script>

<Dialog title={t(agent ? 'ai.editAgent' : 'ai.addAgent')} width={460} onclose={() => { if (!saving) onclose(); }}>
  <label>{t('ai.agentName')}<input bind:value={fields.name} disabled={saving} /></label>
  <label>
    {t('ai.command')}
    <input class="mono" bind:value={fields.command} spellcheck="false" disabled={saving} />
    <small>{t('ai.commandHint')}</small>
  </label>
  <label>
    {t('ai.args')}
    <textarea rows="3" bind:value={fields.args} spellcheck="false" disabled={saving}></textarea>
    <small>{t('ai.argsHint')}</small>
  </label>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#snippet actions()}
    <button type="button" class="secondary" disabled={saving} onclick={onclose}>{t('ai.cancel')}</button>
    <button type="button" class="primary" disabled={saving || !fields.command.trim()} onclick={() => void save()}>{t(agent ? 'ai.save' : 'ai.add')}</button>
  {/snippet}
</Dialog>

<style>
  .mono { font-family: var(--font-mono); }
  textarea {
    padding: var(--space-2);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--fg);
  }
  small { color: var(--fg-muted); }
</style>
