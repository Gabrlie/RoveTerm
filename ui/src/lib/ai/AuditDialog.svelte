<script lang="ts">
  // The audit trail of AI sessions: what each agent was asked, what it was allowed, what ran.
  import { onMount } from 'svelte';
  import Dialog from '../ui/Dialog.svelte';
  import { errorText, t, type MessageKey } from '../i18n';
  import { aiRecords, aiSessions, type AiSessionSummary } from '../ipc';
  import { auditLine, type AuditLine } from './audit';

  let { initial = null, onclose }: { initial?: string | null; onclose: () => void } = $props();

  let sessions = $state<AiSessionSummary[]>([]);
  // svelte-ignore state_referenced_locally
  let selected = $state<string | null>(initial);
  let lines = $state<{ ts: number; line: AuditLine }[]>([]);
  let error = $state('');

  async function load(id: string): Promise<void> {
    selected = id;
    try {
      const records = await aiRecords(id);
      lines = records.flatMap((record) => {
        const line = auditLine(record);
        return line ? [{ ts: record.ts, line }] : [];
      });
      error = '';
    } catch (caught) {
      error = errorText(caught);
    }
  }

  function time(ts: number): string {
    return new Date(ts).toLocaleString();
  }

  /** The note in the user's language where it is a known value. */
  function noteText(line: AuditLine): string {
    const prefix = line.kind === 'approval_asked' ? 'ai.risk.' : line.kind === 'approval_answered' ? 'ai.answer.' : 'ai.result.';
    const key = `${prefix}${line.note}` as MessageKey;
    const text = t(key);
    return text === key ? line.note : text;
  }

  function label(kind: string): string {
    const key = `ai.audit.${kind}` as MessageKey;
    const text = t(key);
    return text === key ? kind : text;
  }

  onMount(() => {
    aiSessions()
      .then((found) => {
        sessions = found;
        const first = selected ?? found[0]?.id;
        if (first) void load(first);
      })
      .catch((caught: unknown) => (error = errorText(caught)));
  });
</script>

<Dialog title={t('ai.history')} {onclose} width={820}>
  <div class="audit">
    <nav aria-label={t('ai.history')}>
      {#each sessions as session (session.id)}
        <button type="button" class:active={session.id === selected} onclick={() => void load(session.id)}>
          <span class="host">{session.host ?? '–'}</span>
          <small>{time(session.started)}{session.agent ? ` · ${session.agent}` : ''}</small>
        </button>
      {:else}
        <p class="empty">{t('ai.historyEmpty')}</p>
      {/each}
    </nav>
    <div class="lines">
      {#if error}<p class="error">{error}</p>{/if}
      {#each lines as { ts, line }, index (index)}
        <div class="line {line.tone}">
          <span class="time">{new Date(ts).toLocaleTimeString()}</span>
          <span class="kind">{label(line.kind)}</span>
          <div class="body">
            {#if line.text}<span class="text selectable">{line.text}</span>{/if}
            {#if line.note}<span class="note">{noteText(line)}</span>{/if}
            {#if line.detail}
              <details><summary>{t('ai.historyDetail')}</summary><pre class="selectable">{line.detail}</pre></details>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  </div>
  {#snippet actions()}
    <button type="button" class="primary" onclick={onclose}>{t('settings.done')}</button>
  {/snippet}
</Dialog>

<style>
  .audit {
    display: flex;
    height: min(560px, 68vh);
    margin: 0 calc(-1 * var(--space-5));
    box-shadow:
      inset 0 1px 0 var(--separator),
      inset 0 -1px 0 var(--separator);
  }
  nav {
    flex: 0 0 220px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2);
    overflow-y: auto;
    box-shadow: inset -1px 0 0 var(--separator);
  }
  nav button {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
  }
  nav button:hover:not(.active) {
    background: var(--fill-hover);
  }
  nav button.active {
    background: var(--accent);
    color: var(--accent-fg);
  }
  nav small {
    opacity: 0.75;
    font-size: var(--text-caption, 11px);
  }
  .host {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    padding: var(--space-3);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .lines {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: var(--space-2) var(--space-4);
    font-size: var(--text-footnote);
  }
  .line {
    display: grid;
    grid-template-columns: 70px 110px 1fr;
    gap: var(--space-2);
    padding: var(--space-1) 0;
    box-shadow: inset 0 -1px 0 var(--separator);
  }
  .time {
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .kind {
    font-weight: 600;
  }
  .line.muted {
    color: var(--fg-muted);
  }
  .line.warning .kind,
  .line.warning .note {
    color: var(--warning);
  }
  .line.danger .kind,
  .line.danger .note {
    color: var(--danger);
  }
  .body {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .text {
    overflow-wrap: anywhere;
    white-space: pre-wrap;
    font-family: var(--font-mono);
  }
  .note {
    color: var(--fg-muted);
  }
  pre {
    margin: var(--space-1) 0 0;
    padding: var(--space-2);
    max-height: 220px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill);
    font-family: var(--font-mono);
    white-space: pre-wrap;
  }
  .error {
    color: var(--danger);
  }
</style>
