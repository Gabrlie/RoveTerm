<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { t } from '../i18n';
  import type { Chat } from './ai.svelte';
  import { subagentStatus } from './subagent';

  let { chats, selected, transcript, attention, onselect, onclose }: {
    chats: Chat[];
    selected: Chat;
    transcript: Snippet<[Chat]>;
    attention: Snippet;
    onselect: (chat: Chat) => void;
    onclose: () => void;
  } = $props();

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
    <div class="heading">
      <span>{t('ai.subagents.title')}</span>
      <strong title={selected.hostLabel ?? ''}>{selected.hostLabel}</strong>
    </div>
    <span class="status {subagentStatus(selected)}">{t(`ai.subagents.status.${subagentStatus(selected)}`)}</span>
    <IconButton icon="close" small label={t('ai.subagents.close')} onclick={onclose} />
  </header>
  {#if chats.length > 1}
    <nav aria-label={t('ai.subagents.title')}>
      {#each chats as sub (sub.id)}
        <button type="button" class:active={selected === sub} aria-pressed={selected === sub} onclick={() => onselect(sub)} title={t(`ai.subagents.status.${subagentStatus(sub)}`)}>
          <span class="dot {subagentStatus(sub)}"></span>
          <span>{sub.hostLabel}</span>
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
    {:else if selected.items.length === 0}
      <p class="muted">{t('ai.subagents.empty')}</p>
    {:else}
      {#key selected.id}
        {@render transcript(selected)}
      {/key}
    {/if}
  </div>
  <footer>{t('ai.subagents.footer')}</footer>
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
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--separator);
  }
  .heading {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: var(--space-1);
  }
  .heading span, footer, .muted {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .heading strong, nav button > span:nth-child(2) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status, .dot { --status: var(--fg-muted); }
  .running { --status: var(--accent); }
  .waiting { --status: var(--warning); }
  .finished { --status: var(--success); }
  .status {
    color: var(--status);
    font-size: var(--text-footnote);
  }
  nav {
    flex-wrap: wrap;
    max-height: 24%;
    overflow: auto;
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--separator);
  }
  nav button {
    max-width: 100%;
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
    background: var(--status);
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
    border-top: 1px solid var(--separator);
    padding: var(--space-2) var(--space-3);
  }
  .error { color: var(--danger); overflow-wrap: anywhere; }
  .retry { align-self: flex-start; }
</style>
