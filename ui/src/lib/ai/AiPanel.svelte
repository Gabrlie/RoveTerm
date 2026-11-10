<script lang="ts">
  // The AI page of the right panel: a conversation with the agent about the focused tab's host.
  // Every operation the agent wants on that host appears as an approval card when the policy
  // asks; what ran appears with its result.
  import { onMount, tick } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Select from '../ui/Select.svelte';
  import Markdown from './Markdown.svelte';
  import AuditDialog from './AuditDialog.svelte';
  import { ai, type ChatItem } from './ai.svelte';
  import { hostOf, workspace } from '../workspace/workspace.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { t, type MessageKey } from '../i18n';
  import type { AiAnswer, AiContext, AiMode, AiOperation, Risk } from '../ipc';

  let { onsettings }: { onsettings: () => void } = $props();

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
  /** The user's terminal on that host, which the agent may read. */
  const terminal = $derived.by(() => {
    const pane = workspace.focusedPane;
    return pane && pane.target.kind !== 'sftp' ? pane.sessionId : null;
  });
  const hostName = $derived.by(() => {
    if (context.kind === 'local') return t('ai.local');
    if (chat.host) return chat.host;
    if (context.kind === 'host') return hosts.find(context.host_id)?.name ?? '';
    return context.address;
  });
  /** The host as part of a sentence. */
  const hostInText = $derived(context.kind === 'local' ? t('ai.localInText') : hostName);

  let draft = $state('');
  let historyOpen = $state(false);
  let list = $state<HTMLDivElement>();
  /** Dangerous approvals the user has acknowledged, by request id. */
  let acknowledged = $state<Record<number, boolean>>({});

  const MODES: { value: AiMode; label: string }[] = [
    { value: 'ask_by_risk', label: t('ai.mode.ask_by_risk') },
    { value: 'ask_all', label: t('ai.mode.ask_all') },
    { value: 'auto', label: t('ai.mode.auto') },
  ];

  function send(): void {
    const text = draft.trim();
    if (!text) return;
    draft = '';
    void chat.send(text, terminal);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      send();
    }
  }

  function answer(item: Extract<ChatItem, { kind: 'approval' }>, choice: AiAnswer): void {
    void chat.answer(item.requestId, choice);
  }

  function describe(operation: AiOperation): string {
    switch (operation.op) {
      case 'command':
        return operation.command;
      case 'read_file':
      case 'write_file':
      case 'list_dir':
        return operation.path;
      case 'read_terminal':
        return t('ai.op.read_terminal');
      case 'agent_tool':
        return operation.command ?? operation.title;
    }
  }

  function opLabel(operation: AiOperation): string {
    return t(`ai.op.${operation.op}` as MessageKey);
  }

  function reason(code: string, detail: string): string {
    const key = `ai.reason.${code}` as MessageKey;
    const text = t(key);
    const label = text === key ? code : text;
    return detail ? `${label}: ${detail}` : label;
  }

  function riskLabel(risk: Risk): string {
    return t(`ai.risk.${risk}` as MessageKey);
  }

  function answerLabel(answer: AiAnswer | null): string {
    return answer === null ? t('ai.answer.withdrawn') : t(`ai.answer.${answer}` as MessageKey);
  }

  // Follow the conversation as it grows.
  $effect(() => {
    void chat.items.length;
    const last = chat.items[chat.items.length - 1];
    if (last && (last.kind === 'agent' || last.kind === 'thought')) void last.text.length;
    void tick().then(() => list?.scrollTo({ top: list.scrollHeight }));
  });

  onMount(() => {
    ai.listen();
    ai.refreshAgents().catch((error: unknown) => console.warn('AI agents unavailable', error));
  });
</script>

<div class="ai">
  <div class="head">
    <span class="host" title={hostName}><Icon name={context.kind === 'local' ? 'terminal' : 'server'} size={13} />{hostName}</span>
    <span class="spacer"></span>
    <div class="mode"><Select label={t('ai.mode')} value={chat.mode} options={MODES} onchange={(mode) => void chat.setMode(mode)} /></div>
    <IconButton icon="history" label={t('ai.history')} small onclick={() => (historyOpen = true)} />
    <IconButton icon="add" label={t('ai.newChat')} small disabled={chat.busy || chat.starting} onclick={() => void ai.reset(chat)} />
  </div>

  {#if chat.plan.length > 0}
    <ol class="plan">
      {#each chat.plan as entry, index (index)}
        <li class={entry.status}>{entry.content}</li>
      {/each}
    </ol>
  {/if}

  <div class="list" bind:this={list}>
    {#if chat.items.length === 0}
      <div class="empty">
        <Icon name="command" size={26} />
        <p>{t('ai.intro', { host: hostInText })}</p>
        {#if ai.agents && ai.agents.agents.length === 0}
          <p class="guide">{t('ai.noAgent')}</p>
          <code class="selectable">npm install -g @agentclientprotocol/codex-acp</code>
          <button type="button" class="secondary" onclick={onsettings}>{t('ai.openSettings')}</button>
        {/if}
      </div>
    {/if}
    {#each chat.items as item, index (index)}
      {#if item.kind === 'user'}
        <div class="user selectable">{item.text}</div>
      {:else if item.kind === 'agent'}
        <div class="agent"><Markdown text={item.text} /></div>
      {:else if item.kind === 'thought'}
        <details class="thought"><summary>{t('ai.thought')}</summary><p class="selectable">{item.text}</p></details>
      {:else if item.kind === 'tool'}
        <div class="tool {item.call.status ?? ''}">
          <Icon name={item.call.status === 'failed' ? 'warning' : item.call.status === 'completed' ? 'check' : 'more'} size={12} />
          <span class="title" title={item.call.locations.join('\n')}>{item.call.title ?? item.call.kind ?? item.call.id}</span>
        </div>
      {:else if item.kind === 'approval'}
        {@const dangerous = item.assessment.risk === 'dangerous'}
        <div class="card approval {item.assessment.risk}" class:done={item.answer !== undefined}>
          <div class="card-head">
            <span class="badge">{riskLabel(item.assessment.risk)}</span>
            <span class="what">{opLabel(item.operation)}</span>
            <span class="where">{item.agentTool ? t('ai.agentTool') : hostName}</span>
          </div>
          <pre class="subject selectable">{describe(item.operation)}</pre>
          {#if item.assessment.reasons.length > 0}
            <ul class="reasons">
              {#each item.assessment.reasons as entry, at (at)}<li>{reason(entry.code, entry.detail === describe(item.operation) ? '' : entry.detail)}</li>{/each}
            </ul>
          {/if}
          {#if item.answer === undefined}
            {#if dangerous}
              <label class="ack"><input type="checkbox" bind:checked={acknowledged[item.requestId]} />{t('ai.acknowledge')}</label>
            {/if}
            <div class="buttons">
              <button type="button" class="secondary" onclick={() => answer(item, 'deny')}>{t('ai.answer.deny')}</button>
              {#if item.rememberable}
                <button type="button" class="secondary" onclick={() => answer(item, 'allow_session')}>{t('ai.answer.allow_session')}</button>
              {/if}
              <button type="button" class="primary" class:danger={dangerous} disabled={dangerous && !acknowledged[item.requestId]} onclick={() => answer(item, 'allow_once')}>{t('ai.answer.allow_once')}</button>
            </div>
          {:else}
            <p class="answered">{answerLabel(item.answer)}</p>
          {/if}
        </div>
      {:else if item.kind === 'operation'}
        {@const outcome = item.outcome}
        <details class="card operation" class:failed={outcome.result === 'failed' || outcome.result === 'denied' || (outcome.result === 'command' && outcome.output.exit_code !== 0)}>
          <summary>
            <span class="what">{opLabel(item.operation)}</span>
            <code class="subject-line">{describe(item.operation)}</code>
            <span class="result">
              {#if outcome.result === 'command'}{t('ai.exit', { code: outcome.output.exit_code ?? '–' })}{:else if outcome.result === 'denied'}{t('ai.answer.deny')}{:else if outcome.result === 'failed'}{t('ai.failed')}{:else}✓{/if}
            </span>
          </summary>
          {#if outcome.result === 'command'}
            {#if outcome.output.stdout}<pre class="selectable">{outcome.output.stdout}</pre>{/if}
            {#if outcome.output.stderr}<pre class="stderr selectable">{outcome.output.stderr}</pre>{/if}
            {#if outcome.output.truncated}<p class="note">{t('ai.truncated')}</p>{/if}
          {:else if outcome.result === 'failed'}
            <p class="note selectable">{outcome.error}</p>
          {:else if outcome.result === 'written'}
            <p class="note">{t('ai.written', { bytes: outcome.bytes, before: outcome.lines_before, after: outcome.lines_after })}</p>
          {/if}
        </details>
      {:else}
        <div class="notice error">
          <p>{item.text}</p>
          {#if item.code === 'ai.no_agent' || item.code === 'ai.spawn' || item.code === 'ai.no_key' || item.code === 'ai.auth_required'}
            <button type="button" class="secondary" onclick={onsettings}>{t('ai.openSettings')}</button>
          {/if}
        </div>
      {/if}
    {/each}
    {#if chat.starting}
      <p class="status">{t('ai.starting')}</p>
    {:else if chat.busy && chat.waiting === 0}
      <p class="status working">{t('ai.working')}</p>
    {/if}
  </div>

  <div class="composer">
    <textarea bind:value={draft} rows="3" placeholder={t('ai.placeholder', { host: hostInText })} onkeydown={onKeydown} aria-label={t('ai.placeholder', { host: hostInText })}></textarea>
    {#if chat.busy}
      <button type="button" class="secondary" onclick={() => void chat.cancel()}>{t('ai.stop')}</button>
    {:else}
      <button type="button" class="primary" disabled={!draft.trim() || chat.starting} onclick={send}>{t('ai.send')}</button>
    {/if}
  </div>
</div>

{#if historyOpen}
  <AuditDialog initial={chat.id} onclose={() => (historyOpen = false)} />
{/if}

<style>
  .ai {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0 var(--space-2) var(--space-2) var(--space-3);
  }
  .host {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .spacer {
    flex: 1;
  }
  .mode {
    width: 128px;
  }
  .plan {
    margin: 0 var(--space-3) var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-5);
    border-radius: var(--radius-sm);
    background: var(--fill);
    font-size: var(--text-footnote);
  }
  .plan .completed {
    color: var(--fg-muted);
    text-decoration: line-through;
  }
  .plan .in_progress {
    font-weight: 600;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3) var(--space-3);
    font-size: var(--text-body);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    margin: auto 0;
    padding: var(--space-4) var(--space-2);
    color: var(--fg-muted);
    text-align: center;
  }
  .empty p {
    margin: 0;
  }
  .empty code {
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
  }
  .user {
    align-self: flex-end;
    max-width: 90%;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--accent);
    color: var(--accent-fg);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .thought summary,
  .status {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .thought p {
    margin: var(--space-1) 0 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    white-space: pre-wrap;
  }
  .status {
    margin: 0;
  }
  .working {
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.4;
    }
  }
  .tool {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .tool.failed {
    color: var(--danger);
  }
  .tool .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--separator);
  }
  .approval {
    --risk: var(--warning);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--risk) 55%, transparent);
  }
  .approval.read_only {
    --risk: var(--success);
  }
  .approval.dangerous {
    --risk: var(--danger);
    background: color-mix(in srgb, var(--danger) 6%, var(--surface));
  }
  .approval.done {
    opacity: 0.7;
    box-shadow: 0 0 0 1px var(--separator);
  }
  .card-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-footnote);
  }
  .badge {
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--risk) 16%, transparent);
    color: var(--risk);
    font-weight: 600;
  }
  .what {
    font-weight: 600;
  }
  .where {
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg-muted);
  }
  .subject,
  pre {
    margin: var(--space-2) 0 0;
    padding: var(--space-2);
    max-height: 240px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill);
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .stderr {
    color: var(--danger);
  }
  .reasons {
    margin: var(--space-2) 0 0;
    padding-left: var(--space-4);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .ack {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
    color: var(--danger);
    font-size: var(--text-footnote);
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-1);
    margin-top: var(--space-2);
  }
  .buttons button {
    height: 26px;
    padding: 0 var(--space-3);
    font-size: var(--text-footnote);
  }
  .primary.danger {
    background: var(--danger);
  }
  .answered,
  .note {
    margin: var(--space-2) 0 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .operation summary {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-footnote);
    cursor: default;
    list-style: none;
  }
  .subject-line {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .result {
    color: var(--success);
  }
  .operation.failed .result {
    color: var(--danger);
  }
  .notice {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-footnote);
  }
  .notice p {
    margin: 0 0 var(--space-1);
  }
  .composer {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3);
    box-shadow: inset 0 1px 0 var(--separator);
  }
  textarea {
    flex: 1;
    min-width: 0;
    resize: none;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--separator);
    color: var(--fg);
    font: inherit;
  }
  textarea:focus {
    outline: none;
    box-shadow: var(--focus-ring);
  }
</style>
