<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Markdown from './Markdown.svelte';
  import AuditDialog from './AuditDialog.svelte';
  import AgentLoginDialog from './AgentLoginDialog.svelte';
  import UsageMeter from './UsageMeter.svelte';
  import ModelPopover from './ModelPopover.svelte';
  import { capabilities, isMobile } from '../platform';
  import { builtinCommands, canPrestart, choiceKey, optionKey, slashItems, slashQuery, splitOptions, type SlashItem } from './composer';
  import { ai, contextKey, type Chat } from './ai.svelte';
  import { formatToolOutput, isLeadTool, LEAD_TOOL_ICONS, OPERATION_ICONS, outcomeState, rows, subject, toolSubject, type ApprovalItem } from './chatview';
  import { hostOf, workspace } from '../workspace/workspace.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { terminals } from '../terminal/registry';
  import { vault } from '../vault/vault.svelte';
  import { t, type MessageKey } from '../i18n';
  import { aiCancel } from '../ipc';
  import type { AgentChoice, AiAnswer, AiContext, AiMode, AiOperation, AiOutcome, AiRecord, AiSessionSummary, ConfigOption, ReviewNote, Risk, ToolCall } from '../ipc';
  import { prefs } from '../settings/prefs.svelte';
  import { EFFORT_LEVELS } from '../settings/providers';
  import { notify } from '../ui/toast.svelte';
  import type { IconName } from '../ui/icons';
  import { ask } from '../ui/confirm.svelte';
  import { ops } from '../ops/ops.svelte';
  import { isTaskActive, shortHostName } from '../ops/taskview';
  import OpsTasks from '../ops/OpsTasks.svelte';
  import SubagentPanel from './SubagentPanel.svelte';
  import { firstWaitingChat, isSubagentActive, latestActivity, subagentStatus, waitingCount } from './subagent';

  /** `fleet`: the fleet lead's conversation on all saved hosts, shown as a page of its own
   *  (a tab), with RoveTerm's assistant only. */
  let { onsettings, fleet = false, onschedules }: { onsettings: () => void; fleet?: boolean; /** The fleet lead's scheduled tasks. */ onschedules?: () => void } = $props();

  type RowState = 'running' | 'done' | 'failed' | 'denied' | 'waiting';

  /** The focused tab's host, or this machine. */
  const context = $derived.by((): AiContext => {
    if (fleet) return { kind: 'fleet' };
    const pane = workspace.focusedPane;
    if (!pane) return { kind: 'local' };
    const hostId = hostOf(pane.target);
    if (hostId) return { kind: 'host', host_id: hostId };
    if (pane.target.kind === 'quick') return { kind: 'quick', address: pane.target.address };
    if (pane.target.kind === 'sftp' && 'address' in pane.target.source) return { kind: 'quick', address: pane.target.source.address };
    return { kind: 'local' };
  });
  /** The user's own conversation about the host. */
  const own = $derived(ai.chat(context));
  /** Conversations agents the user runs have on this host. */
  const externals = $derived(fleet ? [] : ai.externalOn(context));
  /** The outside agent's conversation being looked at, instead of the user's own. */
  let viewing = $state<Chat | null>(null);
  const chat = $derived(viewing && externals.includes(viewing) ? viewing : own);
  const opsConversation = $derived(chat.id ?? chat.resumeId);
  const opsTasksList = $derived(fleet ? ops.tasksOf(opsConversation) : []);
  const subagents = $derived(fleet ? ai.subagentsOf(opsConversation) : []);
  const subSelection = $derived(fleet && opsConversation && ai.requestedSubagent?.parentId === opsConversation ? ai.requestedSubagent : null);
  const selectedSubagent = $derived(subSelection ? ai.subagent(subSelection.id) : undefined);
  const waitingSubagents = $derived(subagents.filter((sub) => sub.waiting > 0));
  const activeSubagents = $derived(subagents.filter(isSubagentActive));
  const hasActiveWork = $derived(fleet && (activeSubagents.length > 0 || ops.hasActive(opsConversation)));
  const activeWorkerCount = $derived(Math.max(activeSubagents.length, ops.activeWorkers(opsConversation)));
  const opsWaiting = $derived(opsTasksList.some((task) => task.counts.waiting_approval > 0));
  const waitingElsewhere = $derived(waitingSubagents.filter((sub) => sub !== selectedSubagent));

  $effect(() => {
    if (fleet && opsConversation) {
      void ops.listen();
      void ops.load(opsConversation);
    }
  });

  function statusOf(sub: Chat) {
    const scopedState = sub.id ? ops.subtaskBySession(sub.parentId, sub.id)?.state : undefined;
    return subagentStatus(sub, scopedState);
  }

  const stoppingSubagents = new SvelteSet<Chat>();

  async function stopSubagent(sub: Chat): Promise<void> {
    if (!sub.id || !isSubagentActive(sub) || stoppingSubagents.has(sub)) return;
    const { id, taskId, subtaskId } = sub;
    stoppingSubagents.add(sub);
    try {
      if (taskId && subtaskId) await ops.cancelSubtask(taskId, subtaskId);
      else await aiCancel(id);
    } catch {
      notify(t('error.ops.failed'), 'error');
    } finally {
      stoppingSubagents.delete(sub);
    }
  }

  function inspectSubagent(id: string, host: string): void {
    if (!opsConversation) return;
    ai.rememberSubagent(opsConversation, id, host);
    ai.openSubagent(opsConversation, id);
  }

  function selectSubagent(sub: Chat): void {
    if (sub.parentId && sub.id) ai.openSubagent(sub.parentId, sub.id);
  }

  function closeSubagents(): void {
    ai.requestedSubagent = null;
  }

  function toggleSubagents(): void {
    if (selectedSubagent) closeSubagents();
    else {
      const sub = firstWaitingChat(subagents) ?? subagents[0];
      if (sub) selectSubagent(sub);
    }
  }
  // Another host shows its own conversation.
  $effect(() => {
    void contextKey(context);
    viewing = null;
  });
  /** The user's terminal on that host, which the agent may read and code can be inserted into. */
  const terminalPane = $derived.by(() => {
    if (fleet) return null;
    const pane = workspace.focusedPane;
    return pane && pane.target.kind !== 'sftp' && pane.sessionId ? pane : null;
  });
  const hostName = $derived.by(() => {
    if (context.kind === 'fleet') return t('fleet.allHosts');
    if (context.kind === 'local') return t('ai.local');
    if (chat.host) return chat.host;
    if (context.kind === 'host') return hosts.find(context.host_id)?.name ?? '';
    return context.address;
  });
  const hostInText = $derived(context.kind === 'local' ? t('ai.localInText') : hostName);

  const agents = $derived((ai.agents?.agents ?? []).filter((entry) => entry.enabled && (!fleet || entry.kind === 'builtin')));
  const agentRef = $derived(chat.agent ?? (fleet ? ai.fleetAgent : ai.defaultAgent));
  const agent = $derived(agents.find((entry) => entry.agent === agentRef) ?? null);
  const waiting = $derived(chat.waiting);
  const provider = $derived(prefs.value.ai.providers.find((entry) => `builtin:${entry.id}` === agentRef));
  const currentModel = $derived(chat.options.find((entry) => entry.id === 'model')?.current ?? chat.selectedModel ?? provider?.model ?? null);
  const windowSize = $derived(provider && currentModel ? provider.contexts[currentModel] ?? provider.context_window : null);

  $effect(() => {
    const target = chat;
    const ref = agentRef;
    if (!canPrestart(ref, !!agent, target)) return;
    const terminal = terminalPane?.sessionId ?? null;
    untrack(() => {
      target.prestartedAgent = ref;
      void target.prepare(terminal);
    });
  });

  let draft = $state('');
  let cancelling = $state(false);
  let creatingChat = $state(false);
  const showStop = $derived(chat.busy || (hasActiveWork && !draft.trim()));
  const statusText = $derived.by(() => {
    if (waiting > 0) return t('ai.waitingApproval', { n: waiting });
    if (chat.starting) return t('ai.starting');
    if (chat.waking) return t('ai.summarizing');
    if (chat.busy) return t('ai.working');
    if (waitingSubagents.length > 0) return t('ai.subagents.waitingSummary', { n: waitingCount(waitingSubagents) });
    if (activeWorkerCount > 0) return t('ai.subagents.runningSummary', { n: activeWorkerCount });
    return t('ai.subagents.tasksRunning');
  });

  async function stopAll(): Promise<void> {
    if (cancelling) return;
    const target = chat;
    const conversation = opsConversation;
    cancelling = true;
    try {
      if (target.id || target.resumeId) await target.cancel();
      const tasks = fleet ? ops.tasksOf(conversation) : [];
      for (const task of tasks) {
        if (isTaskActive(task)) await ops.cancelTask(task.id).catch(() => undefined);
      }
    } finally {
      cancelling = false;
    }
  }

  async function newChat(): Promise<void> {
    if (creatingChat || cancelling) return;
    const target = chat;
    creatingChat = true;
    try {
      if (fleet && hasActiveWork) {
        if (!(await ask({
          title: t('ai.newChat'),
          message: t('ai.newChatConfirmStop'),
          confirmLabel: t('ai.newChatConfirmAction'),
          danger: true,
        })) || target !== chat) return;
        if (hasActiveWork) await stopAll();
      }
      await ai.reset(target);
    } finally {
      creatingChat = false;
    }
  }

  let input = $state<HTMLTextAreaElement>();
  let list = $state<HTMLDivElement>();
  let pickerOpen = $state(false);
  let picker = $state<HTMLDivElement>();
  let agentTrigger = $state<HTMLButtonElement>();
  let modeOpen = $state(false);
  let modePicker = $state<HTMLDivElement>();
  let modeTrigger = $state<HTMLButtonElement>();
  let slashMenu = $state<HTMLDivElement>();
  let historyOpen = $state(false);
  let loginTarget = $state<{ agent: string; name: string } | null>(null);
  /** The title being typed, while renaming. */
  let renaming = $state<string | null>(null);
  $effect(() => {
    void chat;
    renaming = null;
  });
  /** The setting whose choices the `/` menu lists, after it was picked there. */
  let slashSetting = $state<ConfigOption | null>(null);
  /** The highlighted entry of the `/` menu. */
  let slashAt = $state(0);
  /** The input the `/` menu was closed on (Escape), until it changes. */
  let slashClosed = $state<string | null>(null);
  const acknowledged = new SvelteSet<ApprovalItem>();
  /** Rows the user expanded, by key. */
  let expanded = $state<Record<string, boolean>>({});

  const MODES: AiMode[] = ['ask_all', 'ask_by_risk', 'auto'];
  const MODE_ICONS: Record<AiMode, IconName> = { ask_all: 'shieldLock', ask_by_risk: 'shieldCheck', auto: 'shieldAlert' };
  const EXAMPLES: MessageKey[] = ['ai.example.health', 'ai.example.logs', 'ai.example.disk'];
  const FLEET_EXAMPLES: MessageKey[] = ['fleet.example.health', 'fleet.example.updates', 'fleet.example.certs'];
  const GROUPS: { kind: AgentChoice['kind']; title: MessageKey }[] = [
    { kind: 'builtin', title: 'ai.group.builtin' },
    { kind: 'acp', title: 'ai.group.acp' },
  ];

  function send(): void {
    const text = draft.trim();
    if (!text || !chat.canSend || !agent) return;
    draft = '';
    following = true;
    void chat.send(text, terminalPane?.sessionId ?? null);
    void tick().then(grow);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (!event.isComposing && slashKey(event)) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
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

  function answer(target: Chat, item: ApprovalItem, choice: AiAnswer): void {
    acknowledged.delete(item);
    void target.answer(item.requestId, choice);
  }

  async function finishRename(): Promise<void> {
    if (renaming === null) return;
    const title = renaming.trim();
    renaming = null;
    if (title && title !== chat.name) await chat.rename(title);
  }

  function focusSelect(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }

  function onRenameKey(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === 'Enter') {
      event.preventDefault();
      void finishRename();
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      renaming = null;
    }
  }

  /** The agent's settings: as it reports them, or, before the built-in assistant starts, its
   *  provider's models. */
  const settings = $derived.by((): ConfigOption[] => {
    if (!provider || currentModel === null) return chat.options;
    const modelChoices = prefs.value.ai.providers.flatMap((p) => [...new Set([p.model, ...p.models, ...(p.id === provider.id ? [currentModel] : [])])].map((name) => ({
      value: JSON.stringify([p.id, name]), name, description: null, group: p.name
    })));
    return [
      { id: 'model', name: t('ai.model'), category: 'model', kind: 'select', current: JSON.stringify([provider.id, currentModel]), choices: modelChoices },
      { id: 'effort', name: t('ai.effort'), category: 'thought_level', kind: 'select', current: chat.options.find((entry) => entry.id === 'effort')?.current ?? provider.effort, choices: EFFORT_LEVELS.map((level) => ({ value: level, name: level, description: null })) },
    ];
  });
  const split = $derived(splitOptions(settings));
  const commands = $derived(agentRef?.startsWith('builtin:') ? builtinCommands(agentRef) : chat.commands);
  const query = $derived(slashQuery(draft));
  const slashOpen = $derived(query !== null && slashClosed !== draft);
  const slashList = $derived(query === null ? [] : slashItems(query, commands, split.others));

  /** A choice's name; the built-in assistant's effort levels in the user's language. */
  function entryName(option: ConfigOption, entry: { value: string; name: string }): string {
    const key = choiceKey(option, entry.value);
    return key ? t(key) : entry.name;
  }

  /** The option's title, in the user's language when RoveTerm knows the setting. */
  function optionName(option: ConfigOption): string {
    const key = optionKey(option);
    return key ? t(key) : option.name;
  }

  /** Starts a local agent that has not started yet, without a message, so that it lists its
   *  models, settings and commands. Returns whether they can be shown. */
  async function prepareAgent(): Promise<boolean> {
    const target = chat;
    if (!target.id && agentRef?.startsWith('acp:')) {
      if (!agent) return false;
      await target.prepare(terminalPane?.sessionId ?? null);
      return chat === target && !!target.id;
    }
    return true;
  }

  function onInput(): void {
    grow();
    slashAt = 0;
    slashSetting = null;
    if (draft === '/') void prepareAgent();
  }

  function pickSlash(item: SlashItem): void {
    if (item.kind === 'command') {
      draft = `/${item.command.name} `;
      slashSetting = null;
      input?.focus();
    } else {
      slashSetting = item.option;
      slashAt = Math.max(0, item.option.choices.findIndex((choice) => choice.value === chat.optionValue(item.option)));
    }
  }

  function pickSettingChoice(option: ConfigOption, value: string): void {
    void chat.setOption(option.id, value);
    slashSetting = null;
    draft = '';
    input?.focus();
  }

  /** Arrow keys, Enter or Tab and Escape in the `/` menu; returns whether the key was used. */
  function slashKey(event: KeyboardEvent): boolean {
    if (!slashOpen) return false;
    const count = slashSetting ? slashSetting.choices.length : slashList.length;
    if (event.key === 'Escape') {
      if (slashSetting) slashSetting = null;
      else slashClosed = draft;
      return true;
    }
    if (count === 0) return false;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      slashAt = (slashAt + (event.key === 'ArrowDown' ? 1 : count - 1)) % count;
      return true;
    }
    if ((event.key === 'Enter' && !event.shiftKey) || event.key === 'Tab') {
      if (slashSetting) {
        const choice = slashSetting.choices[slashAt];
        if (choice) pickSettingChoice(slashSetting, choice.value);
      } else {
        const item = slashList[slashAt];
        if (item) pickSlash(item);
      }
      return true;
    }
    return false;
  }

  async function resume(summary: AiSessionSummary, records: AiRecord[]): Promise<void> {
    historyOpen = false;
    await ai.resume(summary, records);
    if (summary.context && contextKey(summary.context) !== contextKey(context)) notify(t('ai.resumedElsewhere', { host: summary.host ?? '' }), 'info');
  }

  function chooseAgent(choice: AgentChoice): void {
    pickerOpen = false;
    if (agentRef?.startsWith('builtin:') && choice.kind === 'builtin') {
      const provider = prefs.value.ai.providers.find((p) => `builtin:${p.id}` === choice.agent);
      if (provider && choice.agent !== agentRef) void chat.selectModel(provider.id, provider.model);
    } else void ai.switchAgent(chat, choice.agent);
  }

  function verb(operation: AiOperation): string {
    if (operation.op === 'agent_tool' && operation.title.trim()) return operation.title.trim();
    return t(`ai.verb.${operation.op}` as MessageKey);
  }

  /** Why the auto mode's judge left this to the user. */
  function reviewText(note: ReviewNote): string {
    switch (note.kind) {
      case 'declined':
        return t('ai.why.declined', { reviewer: t(`ai.reviewer.${note.reviewer}` as MessageKey), percent: Math.round(note.confidence * 100) });
      case 'failed':
        return t('ai.why.failed', { reviewer: t(`ai.reviewer.${note.reviewer}` as MessageKey) });
      case 'no_judge':
        return t('ai.why.noJudge');
      case 'opaque':
        return t('ai.why.opaque');
      case 'dangerous':
        return t('ai.why.dangerous');
    }
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
        return outcome.error.includes('could not confirm that the remote command stopped') ? t('ai.remoteExitUnknown') : outcome.error;
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
    const target = event.target as Node;
    if (pickerOpen && picker && !picker.contains(target)) pickerOpen = false;
    if (modeOpen && modePicker && !modePicker.contains(target)) modeOpen = false;
    if (slashOpen && target !== input && !slashMenu?.contains(target)) {
      slashClosed = draft;
      slashSetting = null;
    }
  }

  function onPopoverKey(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing) return;
    if (pickerOpen) {
      pickerOpen = false;
      agentTrigger?.focus();
    } else if (modeOpen) {
      modeOpen = false;
      modeTrigger?.focus();
    } else if (!slashKey(event)) return;
    event.preventDefault();
  }

  onMount(() => {
    ai.listen();
    ai.refreshAgents().catch((error: unknown) => console.warn('AI agents unavailable', error));
  });
</script>

<svelte:window onpointerdown={onWindowPointer} onkeydown={onPopoverKey} />

{#snippet operationRow(key: string, operation: AiOperation, outcome: AiOutcome | null, state: RowState, reviewer?: string)}
  {@const output = outputOf(outcome)}
  <button type="button" class="op {state}" aria-expanded={!!expanded[key]} disabled={!output} onclick={() => toggle(key)}>
    <span class="op-icon"><Icon name={OPERATION_ICONS[operation.op]} size={13} /></span>
    <span class="op-verb" title={verb(operation)}>{verb(operation)}</span>
    {#if subject(operation)}<code class="op-subject" title={subject(operation)}>{subject(operation)}</code>{/if}
    {#if reviewer}<span class="reviewed" title={t('ai.reviewedBy', { reviewer: t(`ai.reviewer.${reviewer}` as MessageKey) })}>{t('ai.reviewed')}</span>{/if}
    <span class="op-state" title={t(`ai.state.${state}` as MessageKey)}><Icon name={stateIcon(state)} size={12} /></span>
  </button>
  {#if expanded[key] && output}<pre class="op-output selectable">{output}</pre>{/if}
{/snippet}

{#snippet toolRow(key: string, call: ToolCall)}
  {@const state = call.status === 'completed' ? 'done' : call.status === 'failed' ? 'failed' : 'running'}
  {@const leadTool = isLeadTool(call.title) ? call.title : null}
  {@const label = leadTool ? t(`ai.tool.${leadTool}`) : call.title ?? call.kind ?? call.id}
  {@const what = toolSubject(call)}
  {@const text = call.content.map((part) => (part.type === 'text' ? part.text : part.type === 'diff' ? `${part.path}\n${part.new_text}` : '')).join('\n')}
  <button type="button" class="op {state}" aria-expanded={!!expanded[key]} disabled={!text} onclick={() => toggle(key)}>
    <span class="op-icon"><Icon name={leadTool ? LEAD_TOOL_ICONS[leadTool] : 'command'} size={13} /></span>
    <span class="op-verb" title={label}>{label}</span>
    {#if what && what !== call.title}<code class="op-subject" title={what}>{what}</code>{/if}
    <span class="op-state" title={t(`ai.state.${state}` as MessageKey)}><Icon name={stateIcon(state)} size={12} /></span>
  </button>
  {#if expanded[key] && text}<pre class="op-output selectable">{formatToolOutput(text)}</pre>{/if}
{/snippet}

{#snippet chatItems(target: Chat)}
    {#each rows(target.items) as row (row.key)}
      {#if row.kind === 'group'}
        {@const key = `${target.key}:g${row.key}`}
        <button type="button" class="op group done" aria-expanded={!!expanded[key]} onclick={() => toggle(key)}>
          <span class="op-icon"><Icon name="chevron" size={12} /></span>
          <span class="op-verb">{t('ai.ranGroup', { n: row.items.length })}</span>
          <span class="op-state"><Icon name="check" size={12} /></span>
        </button>
        {#if expanded[key]}
          <div class="group-items">
            {#each row.items as item, at (at)}
              {#if item.kind === 'operation'}
                {@render operationRow(`${key}-${at}`, item.operation, item.outcome, outcomeState(item.outcome), item.reviewer)}
              {:else}
                {@render toolRow(`${key}-${at}`, item.call)}
              {/if}
            {/each}
          </div>
        {/if}
      {:else}
        {@const item = row.item}
        {@const key = `${target.key}:i${row.key}`}
        {#if item.kind === 'user'}
          <div class="user selectable">{item.text}</div>
        {:else if item.kind === 'delegation'}
          <div class="delegation">
            <span>{t('ai.subagents.delegatedTask')}</span>
            <p class="selectable">{item.task}</p>
          </div>
        {:else if item.kind === 'agent'}
          <div class="agent"><Markdown text={item.text} oninsert={terminalPane ? insert : undefined} /></div>
        {:else if item.kind === 'thought'}
          <details class="thought"><summary>{t('ai.thought')}</summary><p class="selectable">{item.text}</p></details>
        {:else if item.kind === 'tool'}
          {@render toolRow(key, item.call)}
        {:else if item.kind === 'operation'}
          {@render operationRow(key, item.operation, item.outcome, outcomeState(item.outcome), item.reviewer)}
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
              {#if item.operation.op === 'edit_file' || item.operation.op === 'update_notes'}
                <pre class="diff selectable">{#each item.operation.diff.split('\n') as line, at (at)}<span class:removed={line.startsWith('- ')} class:added={line.startsWith('+ ')}>{line}{'\n'}</span>{/each}</pre>
              {/if}
              <p class="approval-where">{item.agentTool ? t('ai.agentTool') : t('ai.on', { host: target.hostLabel ?? hostInText })}</p>
              {#if item.review}<p class="approval-why">{reviewText(item.review)}</p>{/if}
              {#if item.assessment.reasons.length > 0}
                <ul class="reasons">
                  {#each item.assessment.reasons as entry, at (at)}<li>{reason(entry.code, entry.detail, item.operation)}</li>{/each}
                </ul>
              {/if}
              {#if dangerous}
                <label class="ack"><input type="checkbox" bind:checked={() => acknowledged.has(item), (checked) => { if (checked) acknowledged.add(item); else acknowledged.delete(item); }} />{t('ai.acknowledge')}</label>
              {/if}
              <div class="approval-buttons">
                <button type="button" class="secondary" onclick={() => answer(target, item, 'deny')}>{t('ai.answer.deny')}</button>
                {#if item.rememberable}
                  <button type="button" class="secondary" onclick={() => answer(target, item, 'allow_session')}>{t('ai.answer.allow_session')}</button>
                {/if}
                <button type="button" class="primary" class:danger={dangerous} disabled={dangerous && !acknowledged.has(item)} onclick={() => answer(target, item, 'allow_once')}>{t('ai.answer.allow_once')}</button>
              </div>
            </div>
          {:else}
            {@const state = item.answer === 'deny' || item.answer === null ? 'denied' : outcomeState(item.outcome)}
            {@render operationRow(key, item.operation, item.outcome, state)}
          {/if}
        {:else if item.kind === 'compacted'}
          <details class="compacted">
            <summary>{t('ai.compacted')}</summary>
            <p class="selectable">{item.summary}</p>
          </details>
        {:else if item.kind === 'connection'}
          <div class="connection {item.state}">
            <button type="button" class="op" aria-expanded={!!expanded[key]} onclick={() => toggle(key)}>
              <span class="op-icon"><Icon name={item.state === 'recovered' ? 'check' : 'warning'} size={13} /></span>
              <span class="op-verb">{item.state === 'retrying' ? t('ai.connection.retrying', { n: item.retries }) : item.state === 'failed' ? t('ai.connection.failed') : t('ai.connection.recovered')}</span>
            </button>
            {#if item.state === 'failed' && !target.parentId}
              <button type="button" class="link" disabled={target.busy || target.starting} onclick={() => void target.retry(terminalPane?.sessionId ?? null)}>{t('ai.connection.retry')}</button>
            {/if}
          </div>
          {#if expanded[key]}<pre class="op-output selectable">{item.detail}</pre>{/if}
        {:else if item.kind === 'subtask'}
          {@const sub = ai.subagent(item.id)}
          {@const opsSub = ops.subtaskBySession(target.id ?? target.resumeId, item.id)}
          {@const status = sub ? statusOf(sub) : subagentStatus({ historical: true, ended: true, waiting: 0, finalState: null }, opsSub?.state)}
          {@const detail = sub && isSubagentActive(sub) ? latestActivity(sub.items) ?? item.task : opsSub?.summary ?? opsSub?.error ?? sub?.summary ?? item.task}
          <button type="button" class="dispatch {status}" class:active={selectedSubagent?.id === item.id} aria-pressed={selectedSubagent?.id === item.id} onclick={() => inspectSubagent(item.id, item.host)}>
            <span class="dispatch-host" title={item.host}><Icon name="server" size={13} /><strong>{shortHostName(item.host)}</strong></span>
            <span class="dispatch-task" title={detail}>{detail}</span>
            <span class="dispatch-status"><span class="work-dot" class:pulsing={status === 'running'} aria-hidden="true"></span>{t(`ai.subagents.status.${status}`)}</span>
            {#if sub && sub.waiting > 0}<span class="sub-badge" aria-label={t('ai.subagents.waitingCount', { n: sub.waiting })}>{sub.waiting}</span>{/if}
            <span class="inspect">{t('ai.subagents.inspect')}<Icon name="chevron" size={12} /></span>
          </button>
        {:else if item.kind === 'notice'}
          <div class="notice">
            <Icon name="warning" size={13} />
            <span class="selectable">{item.text}</span>
            {#if item.code === 'ai.auth_required' && !isMobile && agentRef?.startsWith('acp:') && target === chat}
              <button type="button" class="link" onclick={() => (loginTarget = { agent: agentRef, name: agent?.name ?? agentRef })}>{t('ai.login.action')}</button>
            {/if}
            {#if item.code === 'ai.no_agent' || item.code === 'ai.spawn' || item.code === 'ai.no_key' || item.code === 'ai.auth_required' || item.code === 'ai.model'}
              <button type="button" class="link" onclick={onsettings}>{t('ai.openSettings')}</button>
            {:else if item.code === 'vault.locked' && !vault.unlocked && !vault.keyLost}
              <button type="button" class="link" onclick={() => void vault.unlock().catch(() => undefined)}>{t('ai.unlock')}</button>
            {/if}
            {#if row.key === target.prepareErrorAt && target === chat}
              <button type="button" class="link" disabled={target.starting} onclick={() => void prepareAgent()}>{t('ai.sidebar.retryStartup')}</button>
            {/if}
          </div>
        {/if}
      {/if}
    {/each}
{/snippet}

{#snippet pendingSubagents(waitingChats: Chat[])}
  {#if waitingChats[0]}
    {@const next = waitingChats[0]}
    <div class="sub-attention" role="status">
      <Icon name="warning" size={14} />
      <span>{t('ai.subagents.pending', { hosts: [...new Set(waitingChats.map((sub) => shortHostName(sub.hostLabel)))].join(', ') })}</span>
      <button type="button" class="secondary" onclick={() => selectSubagent(next)}>{t('ai.subagents.review')}</button>
    </div>
  {/if}
{/snippet}

<div class="ai-split" class:has-subagent={!!selectedSubagent}>
<div class="ai">
  {#if chat.external}
    <div class="toolbar">
      <IconButton icon="back" small label={t('ai.back')} onclick={() => (viewing = null)} />
      <span class="external-title"><Icon name="plug" size={13} />{t('ai.externalTitle', { agent: chat.external })}</span>
      <span class="spacer"></span>
      {#if chat.ended}<IconButton icon="close" small label={t('ai.externalDismiss')} onclick={() => ((viewing = null), ai.dismiss(chat))} />{/if}
    </div>
  {:else}
  <div class="toolbar">
    {#if fleet}
      <span class="pick agent-pick fleet-title"><Icon name="bot" size={13} /><span class="pick-name">{t('fleet.title')}</span></span>
    {:else}
    <div class="picker" bind:this={picker}>
      <button type="button" class="pick agent-pick" bind:this={agentTrigger} onclick={() => (pickerOpen = !pickerOpen)} aria-expanded={pickerOpen} aria-haspopup="menu" title={agent ? `${t('ai.agent')}: ${agent.name}${agent.detail ? `\n${agent.detail}` : ''}` : t('ai.chooseAgent')}>
        <Icon name="bot" size={13} /><span class="pick-name">{agent?.name ?? t('ai.chooseAgent')}</span>
        <span class="pick-arrow"><Icon name="chevronDown" size={11} /></span>
      </button>
      {#if pickerOpen}
        <div class="menu down" role="menu" aria-label={t('ai.agent')}>
          {#each GROUPS as group (group.kind)}
            {@const entries = agents.filter((entry) => entry.kind === group.kind)}
            {#if entries.length > 0}
              <p class="menu-title">{t(group.title)}</p>
              {#each entries as entry (entry.agent)}
                <button type="button" role="menuitemradio" aria-checked={entry.agent === agentRef} class="menu-item" class:current={entry.agent === agentRef} onclick={() => chooseAgent(entry)}>
                  <span class="menu-copy">
                    <span class="menu-name">{entry.name}</span>
                    {#if entry.via_npx || entry.detail}<span class="menu-detail">{entry.via_npx ? t('ai.viaNpx') : entry.detail}</span>{/if}
                  </span>
                  {#if entry.agent === agentRef}<span class="menu-check"><Icon name="check" size={13} /></span>{/if}
                </button>
              {/each}
            {/if}
          {/each}
          <div class="menu-footer">
            <button type="button" role="menuitem" class="menu-item manage" onclick={() => ((pickerOpen = false), onsettings())}><Icon name="settings" size={13} />{t('ai.manageAgents')}</button>
          </div>
        </div>
      {/if}
    </div>
    {/if}
    <span class="divider" aria-hidden="true"></span>
    {#if renaming !== null}
      <input class="rename" use:focusSelect bind:value={renaming} aria-label={t('ai.rename')} maxlength="120" onkeydown={onRenameKey} onblur={() => void finishRename()} />
    {:else}
      <button type="button" class="title" title={t('ai.renameHint')} onclick={() => (renaming = chat.name ?? '')}>{chat.name ?? t('ai.newChatTitle')}</button>
      <span class="spacer"></span>
    {/if}
    <IconButton icon="add" small label={t('ai.newChat')} disabled={chat.starting || (chat.busy && !hasActiveWork) || cancelling || creatingChat} onclick={() => void newChat()} />
    {#if capabilities.schedules && fleet && onschedules}
      <span class="schedules">
        <IconButton icon="clock" small label={t('fleet.schedules')} onclick={onschedules} />
        {#if ai.unreadReports > 0}<span class="dot" aria-label={t('fleet.newReports', { n: ai.unreadReports })}></span>{/if}
      </span>
    {/if}
    {#if fleet && subagents.length > 0}
      <button type="button" class="pick sub-toggle" class:active={!!selectedSubagent} aria-expanded={!!selectedSubagent} aria-label={t('ai.subagents.count', { n: subagents.length })} title={t('ai.subagents.count', { n: subagents.length })} onclick={toggleSubagents}>
        <Icon name="server" size={13} />{subagents.length}
        {#if activeSubagents.length > 0}<span class="work-dot pulsing" aria-hidden="true"></span>{/if}
        {#if waitingSubagents.length > 0}<span class="sub-badge" aria-label={t('ai.subagents.waitingCount', { n: waitingCount(waitingSubagents) })}>{waitingCount(waitingSubagents)}</span>{/if}
      </button>
    {/if}
    <IconButton icon="history" small label={t('ai.history')} onclick={() => (historyOpen = true)} />
    <IconButton icon="settings" small label={t('ai.openSettings')} onclick={onsettings} />
  </div>
  {/if}

  {#if !chat.external && externals.length > 0}
    <div class="externals">
      {#each externals as other (other.key)}
        <button type="button" class="external" onclick={() => (viewing = other)}>
          <Icon name="plug" size={12} />
          <span class="external-name">{other.ended ? t('ai.externalEnded', { agent: other.external ?? '' }) : t('ai.externalWorking', { agent: other.external ?? '' })}</span>
          {#if other.waiting > 0}<span class="external-badge">{other.waiting}</span>{/if}
          <Icon name="chevron" size={11} />
        </button>
      {/each}
    </div>
  {/if}

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

  {#if fleet && opsConversation}<OpsTasks conversation={opsConversation} onselectsubagent={inspectSubagent} />{/if}

  <div class="list" bind:this={list} onscroll={onScroll}>
    {#if chat.items.length === 0}
      <div class="empty">
        <span class="empty-icon"><Icon name="bot" size={28} /></span>
        <p class="empty-title">{fleet ? t('fleet.emptyTitle') : t('ai.emptyTitle', { host: hostInText })}</p>
        {#if agents.length === 0 && ai.agents}
          <p class="empty-text">{fleet ? t('fleet.noProvider') : t('ai.noAgents')}</p>
          <button type="button" class="primary" onclick={onsettings}>{t('ai.setUp')}</button>
        {:else}
          <p class="empty-text">{fleet ? t('fleet.emptyText') : t('ai.emptyText')}</p>
          <div class="examples">
            {#each fleet ? FLEET_EXAMPLES : EXAMPLES as example (example)}
              <button type="button" class="example" onclick={() => ((draft = t(example)), input?.focus())}>{t(example)}</button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    {@render chatItems(chat)}
  </div>

  {#if chat.external}
    <p class="external-note">{chat.ended ? t('ai.externalEndedNote', { agent: chat.external }) : t('ai.externalNote', { agent: chat.external })}</p>
  {:else}
  {@render pendingSubagents(waitingElsewhere)}
  <div class="composer">
    {#if chat.starting || chat.busy || hasActiveWork}
      <p class="status" class:attention={waiting > 0 || waitingSubagents.length > 0 || opsWaiting} role="status">
        <span class="work-dot pulsing" aria-hidden="true"></span>{statusText}
      </p>
    {/if}
    <div class="card">
      <div class="context">
        <span class="chip host" title={hostName}><Icon name={context.kind === 'local' ? 'terminal' : 'server'} size={11} /><span>{hostName}</span></span>
        {#if !fleet}
          <span class="terminal-toggle" title={chat.hasUserMessage ? t('ai.terminalFixed') : t('ai.terminalHint')}>
            <button type="button" class="chip toggle" class:on={chat.attachTerminal} aria-pressed={chat.attachTerminal} aria-label={t('ai.sidebar.terminal')} disabled={!chat.canAttachTerminal} onclick={() => void chat.setAttachTerminal(!chat.attachTerminal, terminalPane?.sessionId ?? null)}>
              <Icon name="terminal" size={11} /><span class="toggle-label">{t('ai.sidebar.terminal')}</span>
            </button>
          </span>
        {/if}
        {#key chat}
          <UsageMeter agent={agentRef} usage={chat.usage} {windowSize} stats={chat.stats} cache={chat.cache} busy={chat.busy} />
        {/key}
      </div>
      {#if slashOpen}
        <div class="menu slash" bind:this={slashMenu} role="listbox" aria-label={t('ai.slash.title')}>
          {#if slashSetting}
            {@const option = slashSetting}
            <p class="menu-title">{optionName(option)}</p>
            {#each option.choices as choice, at (choice.value)}
              <button type="button" role="option" aria-selected={at === slashAt} class="menu-item" class:active={at === slashAt} class:current={choice.value === chat.optionValue(option)} onpointerenter={() => (slashAt = at)} onclick={() => pickSettingChoice(option, choice.value)}>
                <span class="menu-copy">
                  <span class="menu-name">{entryName(option, choice)}</span>
                  {#if choice.description}<span class="menu-detail wrap">{choice.description}</span>{/if}
                </span>
                {#if choice.value === chat.optionValue(option)}<span class="menu-check"><Icon name="check" size={13} /></span>{/if}
              </button>
            {/each}
          {:else if chat.starting}
            <p class="menu-title">{t('ai.starting')}</p>
          {:else if slashList.length === 0}
            <p class="menu-title">{commands.length === 0 && split.others.length === 0 ? t('ai.slash.none') : t('ai.slash.noMatch')}</p>
          {:else}
            {#each slashList as item, at (item.kind === 'command' ? `c:${item.command.name}` : `s:${item.option.id}`)}
              {#if at === 0 || (item.kind === 'setting' && slashList[at - 1]?.kind === 'command')}
                <p class="menu-title">{item.kind === 'command' ? t('ai.slash.commands') : t('ai.slash.settings')}</p>
              {/if}
              <button type="button" role="option" aria-selected={at === slashAt} class="menu-item" class:active={at === slashAt} onpointerenter={() => (slashAt = at)} onclick={() => pickSlash(item)}>
                <span class="menu-copy">
                  {#if item.kind === 'command'}
                    <span class="menu-name"><span class="mono">/{item.command.name}</span>{#if item.command.hint}<span class="hint">{item.command.hint}</span>{/if}</span>
                    {#if item.command.description}<span class="menu-detail wrap">{item.command.description}</span>{/if}
                  {:else}
                    <span class="menu-name">{optionName(item.option)}</span>
                    <span class="menu-detail">{entryName(item.option, item.option.choices.find((choice) => choice.value === chat.optionValue(item.option)) ?? { value: '', name: chat.optionValue(item.option) })}</span>
                  {/if}
                </span>
              </button>
            {/each}
          {/if}
        </div>
      {/if}
      <textarea bind:this={input} bind:value={draft} rows="1" placeholder={fleet ? t('fleet.placeholder') : t('ai.placeholder', { host: hostInText })} aria-label={fleet ? t('fleet.placeholder') : t('ai.placeholder', { host: hostInText })} oninput={onInput} onkeydown={onKeydown}></textarea>
      <div class="card-foot">
        <div class="foot-controls">
          {#key chat}
            <ModelPopover
              model={split.model}
              effort={split.effort}
              toggles={split.toggles}
              speed={split.speed}
              title={optionName}
              value={(option) => chat.optionValue(option)}
              label={entryName}
              onset={(option, value) => {
                if (option === 'model' && agentRef?.startsWith('builtin:')) {
                  const [provider, model] = JSON.parse(value) as [string, string];
                  void chat.selectModel(provider, model);
                } else void chat.setOption(option, value);
              }}
              onopen={prepareAgent}
              starting={chat.starting}
              busy={chat.busy}
            />
          {/key}
          <span class="foot-sep" aria-hidden="true"></span>
          <div class="picker spread mode-picker" bind:this={modePicker}>
            <button type="button" class="mode-trigger {chat.mode}" bind:this={modeTrigger} onclick={() => (modeOpen = !modeOpen)} aria-expanded={modeOpen} aria-haspopup="menu" aria-label={`${t('ai.mode')} · ${t(`ai.sidebar.mode.${chat.mode}`)}`} title={`${t(`ai.sidebar.mode.${chat.mode}`)} — ${t(`ai.modeHint.${chat.mode}`)}`}>
              <span class="mode-icon"><Icon name={MODE_ICONS[chat.mode]} size={12} /></span>
              <span class="mode-label">{t(`ai.sidebar.mode.${chat.mode}`)}</span>
              <span class="mode-arrow"><Icon name="chevronDown" size={10} /></span>
            </button>
            {#if modeOpen}
              <div class="menu" role="menu" aria-label={t('ai.mode')}>
                <p class="menu-title">{t('ai.mode')}</p>
                {#each MODES as mode (mode)}
                  <button type="button" role="menuitemradio" aria-checked={mode === chat.mode} class="menu-item" class:current={mode === chat.mode} onclick={() => ((modeOpen = false), void chat.setMode(mode), modeTrigger?.focus())}>
                    <span class="mode-badge mode-tone {mode}"><Icon name={MODE_ICONS[mode]} size={15} /></span>
                    <span class="menu-copy">
                      <span class="menu-name">{t(`ai.sidebar.mode.${mode}`)}</span>
                      <span class="menu-detail wrap">{t(`ai.modeHint.${mode}`)}</span>
                    </span>
                    {#if mode === chat.mode}<span class="menu-check"><Icon name="check" size={13} /></span>{/if}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        </div>
        {#if showStop}
          <button type="button" class="send stop" disabled={cancelling} title={chat.busy ? t('ai.stop') : t('ai.stopAll')} aria-label={chat.busy ? t('ai.stop') : t('ai.stopAll')} onclick={() => void stopAll()}><Icon name="stop" size={11} /></button>
        {:else}
          <button type="button" class="send" title={t('ai.send')} aria-label={t('ai.send')} disabled={!draft.trim() || !chat.canSend || !agent} onclick={send}><Icon name="up" size={13} /></button>
        {/if}
      </div>
    </div>
  </div>
  {/if}
</div>

  {#if selectedSubagent && subSelection}
    {@const siblings = ai.subagentsOf(subSelection.parentId)}
    <div class="subagent-side">
      <SubagentPanel chats={siblings} selected={selectedSubagent} transcript={chatItems} {statusOf} onstop={(sub) => void stopSubagent(sub)} onselect={selectSubagent} onclose={closeSubagents}>
        {#snippet attention()}
          <div class="overlay-attention">{@render pendingSubagents(siblings.filter((sub) => sub !== selectedSubagent && sub.waiting > 0))}</div>
        {/snippet}
      </SubagentPanel>
    </div>
  {/if}
</div>

{#if historyOpen}
  <AuditDialog role={fleet ? 'fleet' : 'assistant'} initial={chat.id ?? chat.resumeId} onclose={() => (historyOpen = false)} onresume={(summary, records) => void resume(summary, records)} />
{/if}
{#if loginTarget}
  <AgentLoginDialog
    agent={loginTarget.agent}
    agentName={loginTarget.name}
    onclose={() => (loginTarget = null)}
    onsignedin={() => {
      chat.prestartedAgent = null;
      void ai.refreshAgents().catch(() => undefined);
      void prepareAgent();
    }}
    onapikey={onsettings}
  />
{/if}

<style>
  .ai-split {
    position: relative;
    container: ai-split / inline-size;
    display: flex;
    height: 100%;
    min-width: 0;
  }
  .subagent-side {
    flex: 0 0 clamp(340px, 42%, 520px);
    min-width: 0;
    border-inline-start: 1px solid var(--separator);
  }
  .overlay-attention { display: none; }
  @container ai-split (width < 960px) {
    .overlay-attention { display: block; padding-top: var(--space-2); }
    .overlay-attention:empty { display: none; }
    .has-subagent > .ai { visibility: hidden; }
    .subagent-side {
      position: absolute;
      inset: 0;
      z-index: 1;
      border-inline-start: 0;
    }
  }
  .ai {
    flex: 1;
    container: ai / inline-size;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .toolbar {
    position: relative;
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    padding: 0 var(--space-2) var(--space-1) var(--space-3);
  }
  .toolbar :global(.icon-button) {
    flex: none;
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
    font-weight: 500;
    color: var(--fg-muted);
  }
  .title:hover {
    background: var(--fill-hover);
  }
  .rename {
    flex: 1;
    min-height: 0;
    height: 24px;
    padding: 0 6px;
    box-sizing: border-box;
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--accent);
    outline: none;
  }
  .spacer {
    flex: 1 1 0;
    min-width: 0;
  }
  .externals {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0 var(--space-3) var(--space-2);
  }
  .external {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--accent-soft);
    color: var(--accent);
    font: inherit;
    font-size: var(--text-footnote);
    text-align: left;
    cursor: pointer;
  }
  .external:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .external-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .external-badge {
    min-width: 16px;
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--danger);
    color: #fff;
    font-size: 10px;
    font-weight: 600;
    text-align: center;
  }
  .external-title {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-footnote);
    font-weight: 600;
  }
  .external-note {
    margin: 0 var(--space-3) var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
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
  .compacted summary {
    color: var(--fg-subtle);
    font-size: 11px;
    text-align: center;
    list-style: none;
    cursor: default;
  }
  .compacted summary::before,
  .compacted summary::after {
    content: '— ';
  }
  .compacted summary::after {
    content: ' —';
  }
  .compacted p {
    margin: var(--space-1) 0 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    white-space: pre-wrap;
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
    min-width: 0;
    min-height: 24px;
    padding: 2px var(--space-2);
    overflow: hidden;
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
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .reviewed {
    flex: none;
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 10.5px;
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
    flex-wrap: wrap;
    gap: 4px 6px;
    min-width: 0;
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
  .diff {
    margin: var(--space-2) 0 0;
    padding: var(--space-2);
    max-height: 220px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill);
    font-family: var(--font-mono);
    font-size: 11.5px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .diff .removed {
    color: var(--danger);
  }
  .diff .added {
    color: var(--success);
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
  .approval-where,
  .approval-why {
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
  .dispatch {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto auto;
    align-items: center;
    gap: var(--space-1) var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--separator);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    color: var(--fg);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .dispatch:hover, .dispatch.active { background: var(--accent-soft); }
  .dispatch.active { border-color: var(--accent); }
  .dispatch-host, .inspect { display: flex; align-items: center; gap: var(--space-1); }
  .dispatch-host strong, .dispatch-task {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dispatch-task {
    grid-row: 2;
    grid-column: 1 / -1;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .dispatch-status {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-footnote);
    color: var(--fg-muted);
  }
  .dispatch.running .dispatch-status, .inspect { color: var(--accent); }
  .dispatch.waiting .dispatch-status, .dispatch.blocked .dispatch-status { color: var(--warning); }
  .dispatch.completed .dispatch-status { color: var(--success); }
  .dispatch.failed .dispatch-status { color: var(--danger); }
  .dispatch-status .work-dot { background: currentColor; }
  .inspect { grid-column: 4; font-size: var(--text-footnote); }
  .delegation {
    padding: var(--space-2) var(--space-3);
    border-inline-start: 2px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--fill);
    overflow-wrap: anywhere;
  }
  .delegation span { font-size: var(--text-footnote); color: var(--fg-muted); }
  .delegation p { margin: var(--space-1) 0 0; white-space: pre-wrap; }
  .sub-attention {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0 var(--space-3) var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--warning-soft);
    color: var(--fg);
    font-size: var(--text-footnote);
  }
  .sub-attention span { flex: 1; overflow-wrap: anywhere; }
  .sub-attention button { flex: none; }
  .sub-toggle { flex: none; }
  .sub-toggle.active { background: var(--accent-soft); color: var(--accent); }
  .sub-badge {
    flex: none;
    min-width: 16px;
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--danger);
    color: #fff;
    font-size: 10px;
    font-weight: 600;
    text-align: center;
  }
  .notice {
    display: flex;
    align-items: flex-start;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-footnote);
  }
  .notice span {
    flex: 1 1 120px;
    min-width: 0;
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
    min-width: 0;
    padding: 0 var(--space-3) var(--space-3);
  }
  .status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-1);
    color: var(--fg-muted);
    font-size: var(--text-caption);
  }
  .status.attention { color: var(--warning); font-weight: 600; }
  .work-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .pulsing { animation: pulse 1.4s ease-in-out infinite; }
  .attention .work-dot { background: var(--warning); animation: none; }
  @media (prefers-reduced-motion: reduce) { .pulsing { animation: none; } }
  @keyframes pulse {
    50% {
      opacity: 0.4;
    }
  }
  .card {
    position: relative;
    container: card / inline-size;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow:
      0 0 0 0.5px var(--border),
      var(--shadow-sm);
    transition: box-shadow var(--dur-fast) var(--ease-out);
  }
  .card:focus-within {
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 68%, transparent),
      0 0 0 3px color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .context {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--radius-xs);
    background: var(--fill);
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-caption);
    white-space: nowrap;
  }
  .host {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
  }
  .host span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip :global(svg) {
    flex: none;
  }
  .terminal-toggle {
    display: inline-flex;
    flex: none;
  }
  .chip.toggle {
    max-width: 100%;
    min-width: 0;
    overflow: hidden;
    background: transparent;
    color: var(--fg-subtle);
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out);
  }
  .chip.toggle:not(:disabled):not(.on):hover {
    background: var(--fill-hover);
    color: var(--fg-muted);
  }
  .chip.toggle.on {
    background: var(--accent-softer);
    box-shadow: inset 0 0 0 0.5px color-mix(in srgb, var(--accent) 26%, transparent);
    color: var(--accent);
  }
  .chip.toggle.on:not(:disabled):hover {
    background: var(--accent-soft);
  }
  .chip.toggle:disabled {
    opacity: 0.72;
    cursor: default;
  }
  textarea {
    width: 100%;
    min-width: 0;
    min-height: 36px;
    resize: none;
    padding: 2px 2px 4px;
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
    justify-content: space-between;
    gap: 6px;
    min-width: 0;
  }
  .foot-controls {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1 1 0;
    min-width: 0;
  }
  .foot-sep {
    flex: none;
    width: 1px;
    height: 12px;
    margin: 0 2px;
    background: var(--separator);
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
    min-width: 0;
    height: 24px;
    padding: 0 4px;
    overflow: hidden;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }
  .pick:hover {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .pick-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pick-arrow {
    display: flex;
    flex: none;
    color: var(--fg-muted);
  }
  .menu {
    position: absolute;
    bottom: calc(100% + 8px);
    left: 0;
    z-index: 20;
    width: min(100%, 320px);
    max-height: min(340px, 55vh);
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: var(--material-popover);
    box-shadow: var(--glass-edge), var(--shadow-lg);
    font-size: var(--text-footnote);
    animation: pop var(--dur-fast) var(--ease-out);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
  .menu-title {
    margin: 3px var(--space-2) 4px;
    color: var(--fg-muted);
    font-size: 11px;
    font-weight: 600;
  }
  .menu-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 7px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .menu-item:hover,
  .menu-item:focus-visible,
  .menu-item.active {
    background: var(--fill-hover);
  }
  .menu-item.current {
    background: var(--accent-soft);
  }
  .menu-item.current .menu-name {
    font-weight: 600;
  }
  .menu-copy {
    display: grid;
    gap: 2px;
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .menu-check {
    display: flex;
    flex: none;
    color: var(--accent);
  }
  .menu-detail {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg-muted);
    font-size: 11px;
  }
  .menu-detail.wrap {
    white-space: normal;
  }
  .picker.spread {
    position: static;
    flex: none;
  }
  .menu.slash {
    right: 0;
    width: auto;
  }
  .menu.down {
    top: calc(100% + 4px);
    bottom: auto;
    left: var(--space-2);
    right: var(--space-2);
    width: auto;
    max-width: 260px;
    animation: pop-down var(--dur-fast) var(--ease-out);
  }
  @keyframes pop-down {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  .menu-name .mono {
    font-family: var(--font-mono);
  }
  .menu-name .hint {
    margin-left: 6px;
    color: var(--fg-muted);
  }
  .mode-picker {
    flex: none;
  }
  .mode-trigger {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    overflow: hidden;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out);
  }
  .mode-trigger:hover {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .mode-trigger[aria-expanded='true'] {
    background: var(--fill);
    color: var(--fg);
  }
  .mode-icon,
  .mode-arrow {
    display: inline-flex;
    align-items: center;
    flex: none;
  }
  .mode-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mode-arrow {
    color: var(--fg-subtle);
    transition:
      transform var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }
  .mode-trigger:hover .mode-arrow,
  .mode-trigger[aria-expanded='true'] .mode-arrow {
    color: var(--fg-muted);
  }
  .mode-trigger[aria-expanded='true'] .mode-arrow {
    transform: rotate(180deg);
  }
  .mode-trigger.ask_by_risk .mode-icon {
    color: var(--accent);
  }
  .mode-trigger.auto {
    background: color-mix(in srgb, var(--warning) 9%, transparent);
    box-shadow: inset 0 0 0 0.5px color-mix(in srgb, var(--warning) 24%, transparent);
    color: var(--warning);
  }
  .mode-trigger.auto:hover,
  .mode-trigger.auto[aria-expanded='true'] {
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--warning);
  }
  .mode-trigger.auto .mode-arrow {
    color: color-mix(in srgb, var(--warning) 75%, transparent);
  }
  .mode-tone {
    --mode-bg: var(--fill);
    --mode-fg: var(--fg-muted);
    background: var(--mode-bg);
    color: var(--mode-fg);
  }
  .mode-tone.ask_by_risk {
    --mode-bg: var(--accent-soft);
    --mode-fg: var(--accent);
  }
  .mode-tone.auto {
    --mode-bg: var(--warning-soft);
    --mode-fg: var(--warning);
  }
  .mode-badge {
    display: grid;
    place-items: center;
    flex: none;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
  }
  .menu-footer {
    margin-top: var(--space-1);
    padding-top: var(--space-1);
    border-top: 1px solid var(--separator);
  }
  .agent-pick {
    max-width: 160px;
    color: var(--fg);
    font-size: var(--text-footnote);
    font-weight: 600;
  }
  .toolbar .picker {
    position: static;
    flex: 0 1 auto;
    min-width: 0;
  }
  .divider {
    flex: none;
    width: 1px;
    height: 14px;
    margin: 0 2px;
    background: var(--separator);
  }
  .send {
    display: grid;
    place-items: center;
    flex: none;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 50%;
    background: linear-gradient(to bottom, color-mix(in srgb, var(--accent) 92%, #fff), var(--accent));
    box-shadow:
      0 0.5px 1px rgb(0 0 0 / 0.15),
      inset 0 0.5px 0 rgb(255 255 255 / 0.25);
    color: var(--accent-fg);
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-out),
      opacity var(--dur-fast);
  }
  .send:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .send:active:not(:disabled) {
    transform: scale(0.95);
  }
  .send:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .send.stop {
    background: var(--fg);
    box-shadow: none;
    color: var(--surface);
  }
  @container ai (max-width: 260px) {
    .toolbar {
      padding: 0 var(--space-2) var(--space-1);
    }
    .list {
      padding: var(--space-2) var(--space-2) var(--space-3);
    }
    .composer {
      padding: 0 var(--space-2) var(--space-2);
    }
  }
  @container card (max-width: 230px) {
    .toggle-label {
      display: none;
    }
    .chip.toggle {
      padding: 0 5px;
    }
  }
  @container card (max-width: 215px) {
    .mode-label {
      display: none;
    }
    .mode-trigger {
      padding: 0 5px;
    }
  }
  .schedules {
    position: relative;
    display: inline-flex;
  }
  .schedules .dot {
    position: absolute;
    top: 2px;
    right: 2px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    pointer-events: none;
  }
</style>
