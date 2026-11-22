<script lang="ts">
  // The audit trail of AI sessions: what each agent was asked, what it was allowed, what ran.
  import Dialog from '../ui/Dialog.svelte';
  import { errorText, t, type MessageKey } from '../i18n';
  import { aiAuditDetail, aiRecords, aiRename, aiSessions, type AiRecord, type AiSessionSummary } from '../ipc';
  import IconButton from '../ui/IconButton.svelte';
  import { auditLine, type AuditLine } from './audit';
  import { ai } from './ai.svelte';
  import { ask } from '../ui/confirm.svelte';

  interface Props {
    role?: 'fleet' | 'assistant';
    initial?: string | null;
    onclose: () => void;
    /** Continues a conversation in the panel. */
    onresume?: (summary: AiSessionSummary, records: AiRecord[]) => void;
  }

  let { initial = null, role, onclose, onresume }: Props = $props();
  let records = $state<AiRecord[]>([]);
  /** The title being typed for the selected conversation. */
  let renaming = $state<string | null>(null);

  async function rename(): Promise<void> {
    const session = parent;
    const title = renaming?.trim() ?? '';
    renaming = null;
    if (!session || !title || title === session.title) return;
    try {
      session.title = await aiRename(session.id, title);
    } catch (caught) {
      error = errorText(caught);
    }
  }

  let sessions = $state<AiSessionSummary[]>([]);
  // svelte-ignore state_referenced_locally
  let selected = $state<string | null>(initial);
  let loadRevision = 0;
  let lines = $state<{ ts: number; line: AuditLine; reference?: string }[]>([]);
  async function detail(event: Event, row: (typeof lines)[number]): Promise<void> {
    if (!(event.currentTarget instanceof HTMLDetailsElement) || !event.currentTarget.open || !selected || !row.reference || row.line.detail) return;
    const id = selected;
    const revision = loadRevision;
    try {
      const payload = await aiAuditDetail(id, row.reference);
      if (revision !== loadRevision) return;
      const outcome = payload.outcome as Record<string, unknown> | undefined;
      row.line.detail = typeof outcome?.content === 'string' ? outcome.content : JSON.stringify(payload, null, 2);
    } catch (caught) { if (revision === loadRevision) error = errorText(caught); }
  }
  let error = $state('');
  const parent = $derived(sessions.find((s) => s.id === selected || s.children.some((c) => c.id === selected)) ?? null);
  const current = $derived(parent?.children.find((s) => s.id === selected) ?? parent);

  async function load(id: string): Promise<void> {
    const revision = ++loadRevision;
    selected = id;
    renaming = null;
    error = '';
    records = [];
    lines = [];
    try {
      const loaded = await aiRecords(id);
      if (revision !== loadRevision) return;
      records = loaded;
      lines = loaded.flatMap((record) => {
        const line = auditLine(record);
        return line ? [{ ts: record.ts, line, reference: typeof record.payload?.body_ref === 'string' ? record.payload.body_ref : undefined }] : [];
      });
      error = '';
    } catch (caught) {
      if (revision === loadRevision) error = errorText(caught);
    }
  }

  async function resumeParent(): Promise<void> {
    const session = parent;
    if (!session || !onresume) return;
    const revision = loadRevision;
    try {
      const leadRecords = current === session ? records : await aiRecords(session.id);
      if (revision === loadRevision && leadRecords.length) onresume(session, leadRecords);
    } catch (caught) {
      if (revision === loadRevision) error = errorText(caught);
    }
  }

  /** Deletes the selected conversation, here and on the synced devices, after asking. */
  async function remove(): Promise<void> {
    const session = parent;
    if (!session) return;
    const name = session.title ?? session.host ?? '';
    const confirmed = await ask({ title: t('ai.deleteTitle'), message: t('ai.deleteMessage', { name }), confirmLabel: t('ai.delete'), danger: true });
    if (!confirmed) return;
    try {
      await ai.delete(session.id);
    } catch (caught) {
      error = errorText(caught);
      return;
    }
    ++loadRevision;
    const at = sessions.findIndex((entry) => entry.id === session.id);
    sessions = sessions.filter((entry) => entry.id !== session.id);
    records = [];
    lines = [];
    const next = sessions[Math.min(at, sessions.length - 1)];
    selected = null;
    if (next) void load(next.id);
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

  // svelte-ignore state_referenced_locally
  let filter = $state<'fleet' | 'assistant' | 'all'>(role ?? 'all');
  $effect(() => {
    let active = true;
    aiSessions(filter === 'all' ? null : filter).then((found) => {
      if (!active) return;
      sessions = found;
      const first = found.find((entry) => entry.id === initial)?.id ?? found[0]?.id;
      if (first) void load(first);
      else { ++loadRevision; selected = null; records = []; lines = []; }
    }).catch((caught: unknown) => { if (active) error = errorText(caught); });
    return () => { active = false; };
  });
</script>

<Dialog title={t('ai.history')} {onclose} width={820}>
  <div class="audit">
    <nav aria-label={t('ai.history')}>
      <select bind:value={filter} aria-label={t('ai.history')}>
        <option value="all">{t('ai.role.all')}</option>
        <option value="fleet">{t('ai.role.fleet')}</option>
        <option value="assistant">{t('ai.role.assistant')}</option>
      </select>
      {#each sessions as session (session.id)}
        <button type="button" class:active={session.id === parent?.id} onclick={() => void load(session.id)}>
          <span class="host">{session.title ?? session.host ?? '–'}</span>
          <small>{t(`ai.role.${session.role}`)} · {session.title && session.host ? `${session.host} · ` : ''}{time(session.started)}{session.agent ? ` · ${session.agent}` : ''}</small>
        </button>
      {:else}
        <p class="empty">{t('ai.historyEmpty')}</p>
      {/each}
    </nav>
    <div class="lines">
      {#if parent}
        <div class="head">
          {#if renaming !== null}
            <!-- svelte-ignore a11y_autofocus -->
            <input class="field" bind:value={renaming} autofocus maxlength="120" aria-label={t('ai.rename')} onkeydown={(event) => (event.key === 'Enter' ? void rename() : event.key === 'Escape' ? (renaming = null) : undefined)} onblur={() => void rename()} />
          {:else}
            <span>{t(`ai.role.${parent.role}`)}{parent.role === 'assistant' && parent.host ? ` · ${parent.host}` : ''}</span>
            <span class="head-title">{parent.title ?? parent.host ?? '–'}</span>
            <IconButton icon="edit" small label={t('ai.rename')} onclick={() => (renaming = parent?.title ?? '')} />
          {/if}
          <span class="spacer"></span>
          <IconButton icon="remove" small label={t('ai.delete')} onclick={() => void remove()} />
          {#if onresume}
            <button type="button" class="secondary" disabled={!parent.agent_ref || !parent.context || (current === parent && !records.length)} title={parent.agent_ref ? '' : t('ai.resumeUnavailable')} onclick={resumeParent}>{t('ai.resume')}</button>
          {/if}
        </div>
        {#if parent.children.length > 0}
          <div class="switcher" role="group" aria-label={t('ai.subagents.auditViews')}>
            <button type="button" aria-pressed={current === parent} onclick={() => parent && void load(parent.id)}>{t('ai.subagents.lead')}</button>
            {#each parent.children as child (child.id)}
              <button type="button" aria-pressed={current === child} title={time(child.started)} onclick={() => void load(child.id)}><span>{t('ai.role.subagent')}{child.host ? ` · ${child.host}` : ''}</span></button>
            {/each}
          </div>
        {/if}
      {/if}
      {#if error}<p class="error">{error}</p>{/if}
      {#each lines as row (row)}
        {@const {ts, line} = row}
        <div class="line {line.tone}">
          <span class="time">{new Date(ts).toLocaleTimeString()}</span>
          <span class="kind">{current?.role === 'subagent' && line.kind === 'prompt' ? t('ai.subagents.delegatedTask') : label(line.kind)}</span>
          <div class="body">
            {#if line.text}<span class="text selectable">{line.text}</span>{/if}
            {#if line.note}<span class="note">{noteText(line)}</span>{/if}
            {#if line.detail || row.reference}
              <details ontoggle={(event) => void detail(event, row)}><summary>{t('ai.historyDetail')}</summary><pre class="selectable">{line.detail.includes('could not confirm that the remote command stopped') && line.tone === 'danger' ? t('ai.remoteExitUnknown') : line.detail}</pre></details>
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
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) 0 var(--space-2);
    box-shadow: inset 0 -1px 0 var(--separator);
  }
  .switcher {
    display: flex;
    gap: var(--space-2);
    overflow-x: auto;
    padding: var(--space-2) 0;
  }
  .switcher button {
    flex: none;
    border: 1px solid transparent;
    border-radius: var(--radius-pill);
    background: var(--fill);
    color: var(--fg);
    padding: var(--space-1) var(--space-2);
    font: inherit;
    cursor: pointer;
  }
  .switcher button:hover, .switcher button[aria-pressed="true"] {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .switcher button[aria-pressed="true"] { border-color: var(--accent); }
  .head-title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
    font-size: var(--text-body);
  }
  .head .field {
    flex: 1;
  }
  .spacer {
    flex: 1;
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
    grid-template-columns: max-content 100px 1fr;
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
