<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { t } from '../i18n';
  import type { Chat } from './ai.svelte';
  import { shortHostName } from '../ops/taskview';
  import { isSubagentActive, type SubagentStatus } from './subagent';

  let { chats, selected, transcript, attention, statusOf, onstop, onselect, onclose }: {
    chats: Chat[];
    selected: Chat;
    transcript: Snippet<[Chat]>;
    attention: Snippet;
    statusOf: (chat: Chat) => SubagentStatus;
    onstop?: (chat: Chat) => void;
    onselect: (chat: Chat) => void;
    onclose: () => void;
  } = $props();

  const status = $derived(statusOf(selected));
  const active = $derived(isSubagentActive(selected));

  let list: HTMLDivElement;
  let following = true;
  $effect(() => {
    const chat = selected;
    if (chat.historyStatus === 'idle') void chat.loadSubagentHistory();
  });

  function onScroll(): void {
    following = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
  }

  let previous: Chat | undefined;
  $effect(() => {
    const last = selected.items.at(-1);
    if (last?.kind === 'agent' || last?.kind === 'thought') void last.text.length;
    void selected.waiting;
    if (previous !== selected) {
      following = true;
      previous = selected;
    }
    if (following) void tick().then(() => list?.scrollTo({ top: list.scrollHeight }));
  });
</script>

<aside class="subagent-panel" aria-label={t('ai.subagents.title')}>
  <header>
    <span class="back"><IconButton icon="back" small label={t('ai.subagents.back')} onclick={onclose} /></span>
    <div class="heading" title={selected.hostLabel ?? ''}>
      <Icon name="server" size={13} />
      <strong>{shortHostName(selected.hostLabel)}</strong>
    </div>
    <span class="status-pill {status}">{t(`ai.subagents.status.${status}`)}</span>
    {#if active && onstop}
      <IconButton icon="stop" small label={t('ai.subagents.stop')} onclick={() => onstop?.(selected)} />
    {/if}
    <span class="close-action"><IconButton icon="close" small label={t('ai.subagents.close')} onclick={onclose} /></span>
  </header>
  {#if chats.length > 1}
    <nav aria-label={t('ai.subagents.title')}>
      {#each chats as sub (sub.id)}
        {@const hostStatus = statusOf(sub)}
        <button type="button" class:active={selected === sub} aria-pressed={selected === sub} onclick={() => onselect(sub)} title={sub.hostLabel ?? ''}>
          <span class="dot {hostStatus}" role="img" aria-label={t(`ai.subagents.status.${hostStatus}`)}></span>
          <span class="host-tab">{shortHostName(sub.hostLabel)}</span>
          {#if sub.waiting > 0}<span class="count" aria-label={t('ai.subagents.waitingCount', { n: sub.waiting })}>{sub.waiting}</span>{/if}
        </button>
      {/each}
    </nav>
  {/if}
  {@render attention()}
  <div class="transcript" bind:this={list} onscroll={onScroll}>
    {#if selected.historical && (selected.historyStatus === 'idle' || selected.historyStatus === 'loading')}
      <p class="muted" role="status">{t('ai.subagents.loading')}</p>
    {:else if selected.historical && selected.historyStatus === 'error'}
      <p class="error" role="alert">{selected.historyError}</p>
      <button type="button" class="secondary retry" onclick={() => void selected.loadSubagentHistory()}>{t('ai.connection.retry')}</button>
    {:else if active && selected.items.every((item) => item.kind === 'delegation')}
      {#if selected.items.length}{@render transcript(selected)}{/if}
      <p class="live muted" role="status"><span class="dot running" aria-hidden="true"></span>{t('ai.subagents.connecting')}</p>
    {:else if selected.items.length === 0}
      <p class="muted">{t('ai.subagents.empty')}</p>
    {:else}
      {#key selected.id}
        {@render transcript(selected)}
      {/key}
    {/if}
  </div>
  <footer>
    {#if active}
      <span class="live {status}"><span class="dot {status}" aria-hidden="true"></span>{t(`ai.subagents.status.${status}`)}</span>
    {/if}
    <span>{t('ai.subagents.footer')}</span>
  </footer>
</aside>

<style>
  .subagent-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }
  header, nav, nav button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  header {
    min-height: var(--row-height);
    padding: var(--space-1) var(--space-3);
    border-bottom: 1px solid var(--separator);
  }
  .back { display: none; }
  .close-action { display: inline-flex; }
  @container ai-split (width < 960px) {
    .back { display: inline-flex; }
    .close-action { display: none; }
  }
  .heading {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  footer, .muted {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .heading strong, .host-tab {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .running { --status: var(--accent); }
  .waiting, .blocked { --status: var(--warning); }
  .completed { --status: var(--success); }
  .failed { --status: var(--danger); }
  .status-pill {
    flex: none;
    padding: 2px var(--space-2);
    border-radius: var(--radius-pill);
    background: var(--fill);
    color: var(--status, var(--fg-muted));
    font-size: var(--text-caption);
    white-space: nowrap;
  }
  nav {
    flex-wrap: nowrap;
    flex: none;
    overflow-x: auto;
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--separator);
  }
  nav button {
    flex: none;
    max-width: 70%;
    border: 1px solid transparent;
    border-radius: var(--radius-pill);
    padding: var(--space-1) var(--space-2);
    background: var(--fill);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  nav button:hover, nav button.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  nav button.active { border-color: var(--accent); }
  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status, var(--fg-subtle));
  }
  .dot.running { animation: pulse 1.4s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.4; } }
  @media (prefers-reduced-motion: reduce) { .dot.running { animation: none; } }
  .live {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--status, var(--fg-muted));
  }
  .count {
    color: var(--warning);
    font-weight: 600;
  }
  .transcript {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    font-size: var(--text-body);
  }
  .transcript > :global(*) { flex: none; }
  footer {
    display: grid;
    gap: var(--space-1);
    border-top: 1px solid var(--separator);
    padding: var(--space-2) var(--space-3);
  }
  .error { color: var(--danger); overflow-wrap: anywhere; }
  .retry { align-self: flex-start; }
  @media (pointer: coarse) {
    nav {
      touch-action: pan-x;
      scrollbar-width: none;
    }
    nav button {
      min-height: 36px;
      padding: var(--space-1) var(--space-3);
    }
  }
</style>
