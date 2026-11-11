<script lang="ts">
  // The AI page of the right panel, laid out like VSCode's agent chat: the conversation about the
  // focused tab's host, each operation as one line that expands, approvals inline, and a composer
  // card with the agent and approval mode at the bottom.
  import { onMount, tick } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Markdown from './Markdown.svelte';
  import AuditDialog from './AuditDialog.svelte';
  import { ai, contextKey } from './ai.svelte';
  import { outcomeState, rows, subject, toolSubject, type ApprovalItem } from './chatview';
  import { hostOf, workspace } from '../workspace/workspace.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { terminals } from '../terminal/registry';
  import { vault } from '../vault/vault.svelte';
  import { t, type MessageKey } from '../i18n';
  import type { AgentChoice, AiAnswer, AiContext, AiMode, AiOperation, AiOutcome, AiRecord, AiSessionSummary, ConfigOption, Risk } from '../ipc';
  import { prefs } from '../settings/prefs.svelte';
  import { EFFORT_LEVELS } from '../settings/providers';
  import { notify } from '../ui/toast.svelte';
  import type { IconName } from '../ui/icons';

  let { onsettings }: { onsettings: () => void } = $props();

  type RowState = 'running' | 'done' | 'failed' | 'denied' | 'waiting';

  /** The focused tab's host, or this machine. */
  const context = $derived.by((): AiContext => {
    const pane = workspace.focusedPane;
    if (!pane) return { kind: 'local' };
    const hostId = hostOf(pane.target);
    if (hostId) return { kind: 'host', host_id: hostId };
    if (pane.target.kind === 'quick') return { kind: 'quick', address: pane.target.address };
    if (pane.target.kind === 'sftp' && 'address' in pane.target.source) return { kind: 'quick', address: pane.target.source.address };
    return { kind: 'local' };
  });
  const chat = $derived(ai.chat(context));
  /** The user's terminal on that host, which the agent may read and code can be inserted into. */
  const terminalPane = $derived.by(() => {
    const pane = workspace.focusedPane;
    return pane && pane.target.kind !== 'sftp' && pane.sessionId ? pane : null;
  });
  const hostName = $derived.by(() => {
    if (context.kind === 'local') return t('ai.local');
    if (chat.host) return chat.host;
    if (context.kind === 'host') return hosts.find(context.host_id)?.name ?? '';
    return context.address;
  });
  const hostInText = $derived(context.kind === 'local' ? t('ai.localInText') : hostName);

  const agents = $derived((ai.agents?.agents ?? []).filter((entry) => entry.enabled));
  const agentRef = $derived(chat.agent ?? ai.defaultAgent);
  const agent = $derived(agents.find((entry) => entry.agent === agentRef) ?? null);
  const laidOut = $derived(rows(chat.items));
  const waiting = $derived(chat.waiting);

  let draft = $state('');
  let input = $state<HTMLTextAreaElement>();
  let list = $state<HTMLDivElement>();
  let pickerOpen = $state(false);
  let picker = $state<HTMLDivElement>();
  let modeOpen = $state(false);
  let modePicker = $state<HTMLDivElement>();
  let historyOpen = $state(false);
  /** The title being typed, while renaming. */
  let renaming = $state<string | null>(null);
  let settingsOpen = $state(false);
  let settingsPicker = $state<HTMLDivElement>();
  /** Dangerous approvals the user acknowledged, by request id. */
  let acknowledged = $state<Record<number, boolean>>({});
  /** Rows the user expanded, by key. */
  let expanded = $state<Record<string, boolean>>({});

  const MODES: { value: AiMode; label: string }[] = [
    { value: 'ask_all', label: t('ai.mode.ask_all') },
    { value: 'ask_by_risk', label: t('ai.mode.ask_by_risk') },
    { value: 'auto', label: t('ai.mode.auto') },
  ];
  const MODE_ICONS: Record<AiMode, IconName> = { ask_all: 'lock', ask_by_risk: 'eye', auto: 'play' };
  const EXAMPLES: MessageKey[] = ['ai.example.health', 'ai.example.logs', 'ai.example.disk'];
  const GROUPS: { kind: AgentChoice['kind']; title: MessageKey }[] = [
    { kind: 'builtin', title: 'ai.group.builtin' },
    { kind: 'acp', title: 'ai.group.acp' },
  ];

  function send(): void {
    const text = draft.trim();
    if (!text || chat.busy || chat.starting) return;
    draft = '';
    following = true;
    void chat.send(text, terminalPane?.sessionId ?? null);
    void tick().then(grow);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      send();
    }
  }

  /** Grows the input with its text, up to a limit. */
  function grow(): void {
    if (!input) return;
    input.style.height = 'auto';
    input.style.height = `${Math.min(input.scrollHeight, 200)}px`;
  }

  /** Puts `code` into the user's terminal on the host, without pressing Enter. */
  function insert(code: string): void {
    const view = terminalPane ? terminals.get(terminalPane.id) : undefined;
    view?.paste(code.replace(/\n+$/, ''));
  }

  function answer(item: ApprovalItem, choice: AiAnswer): void {
    void chat.answer(item.requestId, choice);
  }

  async function finishRename(): Promise<void> {
    if (renaming === null) return;
    const title = renaming;
    renaming = null;
    if (title.trim() !== (chat.name ?? '')) await chat.rename(title);
  }

  function onRenameKey(event: KeyboardEvent): void {
    if (event.key === 'Enter') void finishRename();
    if (event.key === 'Escape') renaming = null;
  }

  /** The agent's settings: as it reports them, or, before the built-in assistant starts, its
   *  provider's models. */
  const settings = $derived.by((): ConfigOption[] => {
    if (chat.options.length > 0) return chat.options;
    if (!agentRef?.startsWith('builtin:')) return [];
    const provider = prefs.value.ai.providers.find((entry) => `builtin:${entry.id}` === agentRef);
    if (!provider) return [];
    const models = [provider.model, ...provider.models];
    return [
      { id: 'model', name: t('ai.model'), category: 'model', kind: 'select', current: provider.model, choices: models.map((model) => ({ value: model, name: model, description: null })) },
      { id: 'effort', name: t('ai.effort'), category: 'thought_level', kind: 'select', current: provider.effort ?? 'default', choices: ['default', ...EFFORT_LEVELS].map((level) => ({ value: level, name: level, description: null })) },
    ];
  });
  const modelSetting = $derived(settings.find((option) => option.category === 'model') ?? settings[0]);

  function choiceName(option: ConfigOption): string {
    const value = chat.optionValue(option);
    return entryName(option, option.choices.find((entry) => entry.value === value) ?? { value, name: value });
  }

  /** A choice's name; the built-in assistant's effort levels in the user's language. */
  function entryName(option: ConfigOption, entry: { value: string; name: string }): string {
    if (option.id === 'effort' && agentRef?.startsWith('builtin:')) {
      const key = `ai.effort.${entry.value}` as MessageKey;
      if (t(key) !== key) return t(key);
    }
    return entry.name;
  }

  /** The option's title; the built-in assistant's in the user's language. */
  function optionName(option: ConfigOption): string {
    if (!agentRef?.startsWith('builtin:')) return option.name;
    return option.id === 'effort' ? t('ai.effort') : option.id === 'model' ? t('ai.model') : option.name;
  }

  /** Opens the model menu; an agent that has not started yet starts first, to list its models. */
  async function openSettings(): Promise<void> {
    if (settingsOpen) {
      settingsOpen = false;
      return;
    }
    if (!chat.id && agentRef?.startsWith('acp:')) {
      await chat.prepare(terminalPane?.sessionId ?? null);
      if (!chat.id) return;
    }
    settingsOpen = true;
  }

  async function resume(summary: AiSessionSummary, records: AiRecord[]): Promise<void> {
    historyOpen = false;
    await ai.resume(summary, records);
    if (summary.context && contextKey(summary.context) !== contextKey(context)) notify(t('ai.resumedElsewhere', { host: summary.host ?? '' }), 'info');
  }

  function chooseAgent(choice: AgentChoice): void {
    pickerOpen = false;
    void ai.switchAgent(chat, choice.agent);
  }

  function verbIcon(operation: AiOperation): IconName {
    switch (operation.op) {
      case 'command':
      case 'read_terminal':
        return 'terminal';
      case 'read_file':
        return 'file';
      case 'write_file':
        return 'edit';
      case 'list_dir':
        return 'folder';
      case 'agent_tool':
        return 'command';
    }
  }

  function verb(operation: AiOperation): string {
    if (operation.op === 'agent_tool' && operation.title.trim()) return operation.title.trim();
    return t(`ai.verb.${operation.op}` as MessageKey);
  }

  function reason(code: string, detail: string, operation: AiOperation): string {
    const key = `ai.reason.${code}` as MessageKey;
    const text = t(key) === key ? code : t(key);
    return detail && detail !== subject(operation) ? `${text}: ${detail}` : text;
  }

  function riskLabel(risk: Risk): string {
    return t(`ai.risk.${risk}` as MessageKey);
  }

  function outputOf(outcome: AiOutcome | null): string {
    if (!outcome) return '';
    switch (outcome.result) {
      case 'command':
        return [outcome.output.stdout, outcome.output.stderr].filter(Boolean).join('\n') || t('ai.noOutput');
      case 'failed':
        return outcome.error;
      case 'written':
        return t('ai.written', { bytes: outcome.bytes, before: outcome.lines_before, after: outcome.lines_after });
      case 'read':
        return t('ai.readBytes', { bytes: outcome.bytes });
      case 'listed':
        return t('ai.listed', { n: outcome.entries });
      default:
        return '';
    }
  }

  function stateIcon(state: RowState): IconName {
    if (state === 'done') return 'check';
    if (state === 'failed') return 'warning';
    if (state === 'denied') return 'close';
    return 'more';
  }

  function toggle(key: string): void {
    expanded[key] = !expanded[key];
  }

  // Follow the conversation as it grows, unless the user scrolled up to read.
  let following = true;
  function onScroll(): void {
    if (list) following = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
  }
  $effect(() => {
    void chat.items.length;
    const last = chat.items[chat.items.length - 1];
    if (last && (last.kind === 'agent' || last.kind === 'thought')) void last.text.length;
    void waiting;
    if (following) void tick().then(() => list?.scrollTo({ top: list.scrollHeight }));
  });

  function onWindowPointer(event: PointerEvent): void {
    if (pickerOpen && picker && !picker.contains(event.target as Node)) pickerOpen = false;
    if (modeOpen && modePicker && !modePicker.contains(event.target as Node)) modeOpen = false;
    if (settingsOpen && settingsPicker && !settingsPicker.contains(event.target as Node)) settingsOpen = false;
  }

  onMount(() => {
    ai.listen();
    ai.refreshAgents().catch((error: unknown) => console.warn('AI agents unavailable', error));
  });
</script>

<svelte:window onpointerdown={onWindowPointer} />

{#snippet operationRow(key: string, operation: AiOperation, outcome: AiOutcome | null, state: RowState)}
  {@const output = outputOf(outcome)}
  <button type="button" class="op {state}" aria-expanded={!!expanded[key]} disabled={!output} onclick={() => toggle(key)}>
    <span class="op-icon"><Icon name={verbIcon(operation)} size={13} /></span>
    <span class="op-verb" title={verb(operation)}>{verb(operation)}</span>
    {#if subject(operation)}<code class="op-subject" title={subject(operation)}>{subject(operation)}</code>{/if}
    <span class="op-state" title={t(`ai.state.${state}` as MessageKey)}><Icon name={stateIcon(state)} size={12} /></span>
  </button>
  {#if expanded[key] && output}<pre class="op-output selectable">{output}</pre>{/if}
{/snippet}

<div class="ai">
  <div class="toolbar">
    {#if renaming !== null}
      <!-- svelte-ignore a11y_autofocus -->
      <input class="rename" bind:value={renaming} autofocus aria-label={t('ai.rename')} maxlength="120" onkeydown={onRenameKey} onblur={() => void finishRename()} />
    {:else}
      <button type="button" class="title" title={t('ai.renameHint')} onclick={() => (renaming = chat.name ?? '')}>{chat.name ?? t('ai.newChatTitle')}</button>
    {/if}
    <span class="spacer"></span>
    <IconButton icon="add" small label={t('ai.newChat')} disabled={chat.busy || chat.starting} onclick={() => void ai.reset(chat)} />
    <IconButton icon="history" small label={t('ai.history')} onclick={() => (historyOpen = true)} />
    <IconButton icon="settings" small label={t('ai.openSettings')} onclick={onsettings} />
  </div>

  {#if chat.plan.length > 0}
    <details class="plan" open>
      <summary>{t('ai.plan', { done: chat.plan.filter((entry) => entry.status === 'completed').length, total: chat.plan.length })}</summary>
      <ol>
        {#each chat.plan as entry, index (index)}
          <li class={entry.status}><Icon name={entry.status === 'completed' ? 'check' : 'more'} size={11} />{entry.content}</li>
        {/each}
      </ol>
    </details>
  {/if}

  <div class="list" bind:this={list} onscroll={onScroll}>
    {#if chat.items.length === 0}
      <div class="empty">
        <span class="empty-icon"><Icon name="bot" size={28} /></span>
        <p class="empty-title">{t('ai.emptyTitle', { host: hostInText })}</p>
        {#if agents.length === 0 && ai.agents}
          <p class="empty-text">{t('ai.noAgents')}</p>
          <button type="button" class="primary" onclick={onsettings}>{t('ai.setUp')}</button>
        {:else}
          <p class="empty-text">{t('ai.emptyText')}</p>
          <div class="examples">
            {#each EXAMPLES as example (example)}
              <button type="button" class="example" onclick={() => ((draft = t(example)), input?.focus())}>{t(example)}</button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    {#each laidOut as row (row.key)}
      {#if row.kind === 'group'}
        {@const key = `g${row.key}`}
        <button type="button" class="op group done" aria-expanded={!!expanded[key]} onclick={() => toggle(key)}>
          <span class="op-icon"><Icon name="chevron" size={12} /></span>
          <span class="op-verb">{t('ai.ranGroup', { n: row.items.length })}</span>
          <span class="op-state"><Icon name="check" size={12} /></span>
        </button>
        {#if expanded[key]}
          <div class="group-items">
            {#each row.items as item, at (at)}
              {@render operationRow(`${key}-${at}`, item.operation, item.outcome, outcomeState(item.outcome))}
            {/each}
          </div>
        {/if}
      {:else}
        {@const item = row.item}
        {@const key = `i${row.key}`}
        {#if item.kind === 'user'}
          <div class="user selectable">{item.text}</div>
        {:else if item.kind === 'agent'}
          <div class="agent"><Markdown text={item.text} oninsert={terminalPane ? insert : undefined} /></div>
        {:else if item.kind === 'thought'}
          <details class="thought"><summary>{t('ai.thought')}</summary><p class="selectable">{item.text}</p></details>
        {:else if item.kind === 'tool'}
          {@const state = item.call.status === 'completed' ? 'done' : item.call.status === 'failed' ? 'failed' : 'running'}
          {@const what = toolSubject(item.call)}
          {@const text = item.call.content.map((part) => (part.type === 'text' ? part.text : part.type === 'diff' ? `${part.path}\n${part.new_text}` : '')).join('\n')}
          <button type="button" class="op {state}" aria-expanded={!!expanded[key]} disabled={!text} onclick={() => toggle(key)}>
            <span class="op-icon"><Icon name="command" size={13} /></span>
            <span class="op-verb" title={item.call.title ?? ''}>{item.call.title ?? item.call.kind ?? item.call.id}</span>
            {#if what && what !== item.call.title}<code class="op-subject" title={what}>{what}</code>{/if}
            <span class="op-state"><Icon name={stateIcon(state)} size={12} /></span>
          </button>
          {#if expanded[key] && text}<pre class="op-output selectable">{text}</pre>{/if}
        {:else if item.kind === 'operation'}
          {@render operationRow(key, item.operation, item.outcome, outcomeState(item.outcome))}
        {:else if item.kind === 'approval'}
          {@const dangerous = item.assessment.risk === 'dangerous'}
          {#if item.answer === undefined}
            <div class="approval {item.assessment.risk}">
              <div class="approval-head">
                <Icon name="warning" size={13} />
                <span class="approval-title">{t('ai.approve', { verb: verb(item.operation) })}</span>
                <span class="badge">{riskLabel(item.assessment.risk)}</span>
              </div>
              {#if subject(item.operation)}<pre class="approval-subject selectable">{subject(item.operation)}</pre>{/if}
              {#if item.operation.op === 'agent_tool' && item.operation.detail}<p class="approval-detail selectable">{item.operation.detail}</p>{/if}
              <p class="approval-where">{item.agentTool ? t('ai.agentTool') : t('ai.on', { host: hostInText })}</p>
              {#if item.assessment.reasons.length > 0}
                <ul class="reasons">
                  {#each item.assessment.reasons as entry, at (at)}<li>{reason(entry.code, entry.detail, item.operation)}</li>{/each}
                </ul>
              {/if}
              {#if dangerous}
                <label class="ack"><input type="checkbox" bind:checked={acknowledged[item.requestId]} />{t('ai.acknowledge')}</label>
              {/if}
              <div class="approval-buttons">
                <button type="button" class="secondary" onclick={() => answer(item, 'deny')}>{t('ai.answer.deny')}</button>
                {#if item.rememberable}
                  <button type="button" class="secondary" onclick={() => answer(item, 'allow_session')}>{t('ai.answer.allow_session')}</button>
                {/if}
                <button type="button" class="primary" class:danger={dangerous} disabled={dangerous && !acknowledged[item.requestId]} onclick={() => answer(item, 'allow_once')}>{t('ai.answer.allow_once')}</button>
              </div>
            </div>
          {:else}
            {@const state = item.answer === 'deny' || item.answer === null ? 'denied' : outcomeState(item.outcome)}
            {@render operationRow(key, item.operation, item.outcome, state)}
          {/if}
        {:else if item.kind === 'connection'}
          <div class="connection {item.state}">
            <button type="button" class="op" aria-expanded={!!expanded[key]} onclick={() => toggle(key)}>
              <span class="op-icon"><Icon name={item.state === 'recovered' ? 'check' : 'warning'} size={13} /></span>
              <span class="op-verb">{item.state === 'retrying' ? t('ai.connection.retrying', { n: item.retries }) : item.state === 'failed' ? t('ai.connection.failed') : t('ai.connection.recovered')}</span>
            </button>
            {#if item.state === 'failed'}
              <button type="button" class="link" disabled={chat.busy || chat.starting} onclick={() => void chat.retry(terminalPane?.sessionId ?? null)}>{t('ai.connection.retry')}</button>
            {/if}
          </div>
          {#if expanded[key]}<pre class="op-output selectable">{item.detail}</pre>{/if}
        {:else}
          <div class="notice">
            <Icon name="warning" size={13} />
            <span class="selectable">{item.text}</span>
            {#if item.code === 'ai.no_agent' || item.code === 'ai.spawn' || item.code === 'ai.no_key' || item.code === 'ai.auth_required' || item.code === 'ai.model'}
              <button type="button" class="link" onclick={onsettings}>{t('ai.openSettings')}</button>
            {:else if item.code === 'vault.locked' && !vault.unlocked && !vault.keyLost}
              <button type="button" class="link" onclick={() => void vault.unlock().catch(() => undefined)}>{t('ai.unlock')}</button>
            {/if}
          </div>
        {/if}
      {/if}
    {/each}
  </div>

  <div class="composer">
    {#if chat.starting || chat.busy}
      <p class="status" class:attention={waiting > 0}>
        {#if waiting > 0}{t('ai.waitingApproval', { n: waiting })}{:else if chat.starting}{t('ai.starting')}{:else}<span class="dots">{t('ai.working')}</span>{/if}
      </p>
    {/if}
    <div class="card">
      <div class="context">
        <span class="chip"><Icon name={context.kind === 'local' ? 'terminal' : 'server'} size={11} />{hostName}</span>
        <label class="chip toggle icon-only" class:on={chat.attachTerminal} title={`${t('ai.terminalHint')}${chat.id ? `\n${t('ai.terminalFixed')}` : ''}`}>
          <input type="checkbox" bind:checked={chat.attachTerminal} disabled={!!chat.id} aria-label={t('ai.terminalHint')} />
          <Icon name="terminal" size={12} />
        </label>
      </div>
      <textarea bind:this={input} bind:value={draft} rows="1" placeholder={t('ai.placeholder', { host: hostInText })} aria-label={t('ai.placeholder', { host: hostInText })} oninput={grow} onkeydown={onKeydown}></textarea>
      <div class="card-foot">
        <div class="picker" bind:this={picker}>
          <button type="button" class="pick" onclick={() => (pickerOpen = !pickerOpen)} aria-expanded={pickerOpen} title={agent?.detail ?? ''}>
            <Icon name="bot" size={12} /><span class="pick-name">{agent?.name ?? t('ai.chooseAgent')}</span><Icon name="chevronDown" size={11} />
          </button>
          {#if pickerOpen}
            <div class="menu" role="menu">
              {#each GROUPS as group (group.kind)}
                {@const entries = agents.filter((entry) => entry.kind === group.kind)}
                {#if entries.length > 0}
                  <p class="menu-title">{t(group.title)}</p>
                  {#each entries as entry (entry.agent)}
                    <button type="button" role="menuitem" class="menu-item" class:current={entry.agent === agentRef} onclick={() => chooseAgent(entry)}>
                      <span class="menu-name">{entry.name}</span>
                      <span class="menu-detail">{entry.via_npx ? t('ai.viaNpx') : entry.detail}</span>
                    </button>
                  {/each}
                {/if}
              {/each}
              <button type="button" role="menuitem" class="menu-item manage" onclick={() => ((pickerOpen = false), onsettings())}>{t('ai.manageAgents')}</button>
            </div>
          {/if}
        </div>
        {#if modelSetting || (agentRef?.startsWith('acp:') && !chat.id)}
          <div class="picker spread" bind:this={settingsPicker}>
            <button type="button" class="pick" onclick={() => void openSettings()} aria-expanded={settingsOpen} title={t('ai.modelSettings')} disabled={chat.starting}>
              <span class="pick-name">{chat.starting ? t('ai.loadingModels') : modelSetting ? choiceName(modelSetting) : t('ai.model')}</span><Icon name="chevronDown" size={11} />
            </button>
            {#if settingsOpen}
              <div class="menu" role="menu" aria-label={t('ai.modelSettings')}>
                {#if settings.length === 0}<p class="menu-title">{t('ai.noModelSettings')}</p>{/if}
                {#each settings as option (option.id)}
                  {#if option.kind === 'boolean'}
                    {@const on = chat.optionValue(option) === 'true'}
                    <button type="button" role="menuitemcheckbox" aria-checked={on} class="menu-item toggle-row" onclick={() => void chat.setOption(option.id, on ? 'false' : 'true')}>
                      <span class="menu-name">{option.name}</span>
                      <span class="switch-mark" class:on></span>
                    </button>
                  {:else}
                  <p class="menu-title">{optionName(option)}</p>
                  {#each option.choices as entry (entry.value)}
                    <button type="button" role="menuitemradio" aria-checked={entry.value === chat.optionValue(option)} class="menu-item" class:current={entry.value === chat.optionValue(option)} onclick={() => ((settingsOpen = false), void chat.setOption(option.id, entry.value))}>
                      <span class="menu-name">{entryName(option, entry)}</span>
                      {#if entry.description}<span class="menu-detail wrap">{entry.description}</span>{/if}
                    </button>
                  {/each}
                  {/if}
                {/each}
              </div>
            {/if}
          </div>
        {/if}
        <div class="picker spread" bind:this={modePicker}>
          <button type="button" class="pick" onclick={() => (modeOpen = !modeOpen)} aria-expanded={modeOpen} title={`${t('ai.mode')}: ${t(`ai.mode.${chat.mode}`)}`} aria-label={`${t('ai.mode')}: ${t(`ai.mode.${chat.mode}`)}`}>
            <Icon name={MODE_ICONS[chat.mode]} size={12} /><Icon name="chevronDown" size={11} />
          </button>
          {#if modeOpen}
            <div class="menu" role="menu" aria-label={t('ai.mode')}>
              <p class="menu-title">{t('ai.mode')}</p>
              {#each MODES as entry (entry.value)}
                <button type="button" role="menuitemradio" aria-checked={entry.value === chat.mode} class="menu-item" class:current={entry.value === chat.mode} onclick={() => ((modeOpen = false), void chat.setMode(entry.value))}>
                  <span class="menu-name with-icon"><Icon name={MODE_ICONS[entry.value]} size={12} />{entry.label}</span>
                  <span class="menu-detail wrap">{t(`ai.modeHint.${entry.value}`)}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
        <span class="spacer"></span>
        {#if chat.busy}
          <button type="button" class="send stop" title={t('ai.stop')} aria-label={t('ai.stop')} onclick={() => void chat.cancel()}><Icon name="stop" size={12} /></button>
        {:else}
          <button type="button" class="send" title={t('ai.send')} aria-label={t('ai.send')} disabled={!draft.trim() || chat.starting || !agent} onclick={send}><Icon name="up" size={14} /></button>
        {/if}
      </div>
    </div>
  </div>
</div>

{#if historyOpen}
  <AuditDialog initial={chat.id ?? chat.resumeId} onclose={() => (historyOpen = false)} onresume={(summary, records) => void resume(summary, records)} />
{/if}

<style>
  .ai {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 var(--space-2) var(--space-1) var(--space-3);
  }
  .title,
  .rename {
    min-width: 0;
    max-width: 100%;
    height: 22px;
    padding: 0 4px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-footnote);
    font-weight: 600;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title {
    cursor: text;
  }
  .title:hover {
    background: var(--fill-hover);
  }
  .rename {
    flex: 1;
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--accent);
    outline: none;
  }
  .spacer {
    flex: 1;
  }
  .plan {
    margin: 0 var(--space-3) var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    box-shadow: 0 0 0 1px var(--separator);
    font-size: var(--text-footnote);
  }
  .plan summary {
    color: var(--fg-muted);
    cursor: default;
  }
  .plan ol {
    margin: var(--space-1) 0 0;
    padding: 0;
    list-style: none;
  }
  .plan li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 1px 0;
  }
  .plan li.completed {
    color: var(--fg-muted);
    text-decoration: line-through;
  }
  .plan li.in_progress {
    font-weight: 600;
  }
  .list {
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow-x: hidden;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3);
    font-size: var(--text-body);
  }
  /* The list scrolls; its rows keep their own height. */
  .list > :global(*) {
    flex: none;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    margin: auto 0;
    padding: var(--space-4) var(--space-2);
    text-align: center;
  }
  .empty-icon {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    border-radius: 14px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .empty-title {
    margin: 0;
    font-weight: 600;
  }
  .empty-text {
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .examples {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
    margin-top: var(--space-1);
  }
  .example {
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-footnote);
    text-align: left;
    cursor: pointer;
  }
  .example:hover {
    background: var(--fill-hover);
  }
  .user {
    align-self: flex-end;
    max-width: 92%;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .thought summary {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    cursor: default;
  }
  .thought p {
    margin: var(--space-1) 0 0;
    padding-left: var(--space-3);
    box-shadow: inset 2px 0 0 var(--separator);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    white-space: pre-wrap;
  }
  .op {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-height: 24px;
    padding: 2px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    text-align: left;
    cursor: pointer;
  }
  .op:hover:not(:disabled) {
    background: var(--fill-hover);
  }
  .op:disabled {
    cursor: default;
  }
  .op-icon {
    display: flex;
    flex: none;
  }
  .op-verb {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg);
  }
  .op-subject {
    flex: 1 1 0;
    min-width: 3em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .op-state {
    display: flex;
    flex: none;
    margin-left: auto;
  }
  .op.done .op-state {
    color: var(--success);
  }
  .op.failed .op-state,
  .op.denied .op-state {
    color: var(--danger);
  }
  .op.denied .op-subject {
    text-decoration: line-through;
  }
  .op.running .op-state {
    animation: pulse 1.2s ease-in-out infinite;
  }
  .op.group[aria-expanded='true'] .op-icon :global(svg) {
    transform: rotate(90deg);
  }
  .group-items {
    padding-left: var(--space-3);
  }
  .op-output {
    margin: 0 0 0 var(--space-5);
    padding: var(--space-2);
    max-height: 260px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill);
    font-family: var(--font-mono);
    font-size: 11.5px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .approval {
    --risk: var(--warning);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--risk) 7%, var(--surface));
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--risk) 45%, transparent);
  }
  .approval.read_only {
    --risk: var(--accent);
  }
  .approval.dangerous {
    --risk: var(--danger);
  }
  .approval-head {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--risk);
  }
  .approval-title {
    min-width: 0;
    overflow-wrap: anywhere;
    color: var(--fg);
    font-weight: 600;
    font-size: var(--text-footnote);
  }
  .badge {
    margin-left: auto;
    padding: 0 6px;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--risk) 16%, transparent);
    color: var(--risk);
    font-size: 11px;
    font-weight: 600;
  }
  .approval-subject {
    margin: var(--space-2) 0 0;
    padding: var(--space-2);
    max-height: 180px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill);
    font-family: var(--font-mono);
    font-size: 11.5px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .approval-detail {
    margin: var(--space-2) 0 0;
    max-height: 160px;
    overflow: auto;
    color: var(--fg-muted);
    font-size: 11.5px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .connection {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    color: var(--warning);
  }
  .connection .op {
    flex: 1;
    min-width: 0;
    color: inherit;
  }
  .connection .op-verb {
    white-space: normal;
    color: inherit;
  }
  .connection.failed {
    color: var(--danger);
  }
  .connection.recovered {
    color: var(--fg-muted);
  }
  .approval-where {
    margin: var(--space-1) 0 0;
    color: var(--fg-muted);
    font-size: 11px;
  }
  .reasons {
    margin: var(--space-1) 0 0;
    padding-left: var(--space-4);
    color: var(--fg-muted);
    font-size: 11px;
  }
  .ack {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
    color: var(--danger);
    font-size: 11px;
  }
  .approval-buttons {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-1);
    margin-top: var(--space-2);
  }
  .approval-buttons button {
    height: 24px;
    padding: 0 var(--space-3);
    font-size: var(--text-footnote);
  }
  .primary.danger {
    background: var(--danger);
  }
  .notice {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-footnote);
  }
  .notice span {
    flex: 1;
    overflow-wrap: anywhere;
  }
  .link {
    flex: none;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .composer {
    padding: 0 var(--space-3) var(--space-3);
  }
  .status {
    margin: 0 0 var(--space-1);
    color: var(--fg-muted);
    font-size: 11px;
  }
  .status.attention {
    color: var(--warning);
    font-weight: 600;
  }
  .dots {
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.4;
    }
  }
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--separator);
  }
  .card:focus-within {
    box-shadow: 0 0 0 1px var(--accent);
  }
  .context {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    height: 20px;
    padding: 0 6px;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip.toggle {
    cursor: pointer;
    opacity: 0.6;
  }
  .chip.toggle.on {
    opacity: 1;
    color: var(--fg);
  }
  .chip.toggle input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .chip.icon-only {
    position: relative;
    width: 22px;
    justify-content: center;
    padding: 0;
  }
  .chip.toggle:not(.on) :global(svg) {
    opacity: 0.45;
  }
  textarea {
    width: 100%;
    min-height: 36px;
    resize: none;
    padding: var(--space-1) 2px;
    border: none;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    color: var(--fg);
    font: inherit;
    outline: none;
  }
  .card-foot {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
  }
  .card-foot .spacer {
    min-width: 0;
  }
  .picker {
    position: relative;
    flex: 0 1 auto;
    min-width: 0;
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    height: 24px;
    padding: 0 4px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: 11.5px;
    cursor: pointer;
  }
  .pick:hover {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .pick-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu {
    position: absolute;
    bottom: calc(100% + 6px);
    left: 0;
    z-index: 20;
    width: 240px;
    max-height: 320px;
    overflow-y: auto;
    padding: var(--space-1);
    border-radius: var(--radius-md);
    background: var(--material-popover);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--glass-edge), var(--shadow-md);
  }
  .menu-title {
    margin: var(--space-1) var(--space-2) 2px;
    color: var(--fg-muted);
    font-size: 11px;
    font-weight: 600;
  }
  .menu-item {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    width: 100%;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .menu-item:hover {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .menu-item.current .menu-name::after {
    content: ' ✓';
  }
  .menu-detail {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0.7;
    font-size: 11px;
  }
  /* Spans the input card rather than starting at its button. */
  .picker.spread {
    position: static;
  }
  .picker.spread .menu {
    right: 0;
    width: auto;
    bottom: calc(100% + 6px);
  }
  .toggle-row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }
  .switch-mark {
    position: relative;
    width: 26px;
    height: 15px;
    border-radius: 8px;
    background: var(--fill-hover);
    transition: background var(--dur-fast) var(--ease-out);
  }
  .switch-mark::after {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 0.5px 1px rgb(0 0 0 / 0.25);
    transition: transform var(--dur-fast) var(--ease-out);
  }
  .switch-mark.on {
    background: var(--success);
  }
  .switch-mark.on::after {
    transform: translateX(11px);
  }
  .menu-detail.wrap {
    white-space: normal;
  }
  .with-icon {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .menu-item.manage {
    margin-top: var(--space-1);
    box-shadow: inset 0 1px 0 var(--separator);
    font-size: var(--text-footnote);
  }
  .send {
    display: grid;
    place-items: center;
    flex: none;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 50%;
    background: var(--accent);
    color: var(--accent-fg);
    cursor: pointer;
  }
  .send:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .send.stop {
    background: var(--fg);
    color: var(--surface);
  }
</style>
