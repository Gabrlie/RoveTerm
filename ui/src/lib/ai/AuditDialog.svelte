<script lang="ts">
  import Dialog from '../ui/Dialog.svelte';
  import { errorText, t, type MessageKey } from '../i18n';
  import { aiAuditDetail, aiRecords, aiSessions, type AiOperation, type AiRecord, type AiSessionSummary } from '../ipc';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import type { IconName } from '../ui/icons';
  import { viewport } from '../layout/viewport.svelte';
  import { pushBackHandler } from '../ui/backStack';
  import { initialCompactDetail, masterDetailView } from '../ui/masterDetail';
  import { shortHostName } from '../ops/taskview';
  import { auditLine, filterSessions, formatAuditDetail, formatSessionTime, groupSessionsByDate, type AuditLine, type FormattedAuditDetail } from './audit';
  import { isLeadTool, itemsFromRecords, LEAD_TOOL_ICONS, OPERATION_ICONS, outcomeState, rows, subject, toolSubject, type ApprovalItem, type GroupItem } from './chatview';
  import { ai } from './ai.svelte';
  import Markdown from './Markdown.svelte';
  import { ask } from '../ui/confirm.svelte';

  interface Props {
    role?: 'fleet' | 'assistant';
    initial?: string | null;
    onclose: () => void;
    /** Continues a conversation in the panel. */
    onresume?: (summary: AiSessionSummary, records: AiRecord[]) => void;
  }

  type DetailEntry = { loading: true } | { detail: FormattedAuditDetail } | { error: string };

  const filters = ['all', 'fleet', 'assistant'] as const;
  const views = ['conversation', 'audit'] as const;
  const recordKinds: Record<AiOperation['op'], string> = {
    command: 'command',
    read_file: 'file_read',
    write_file: 'file_write',
    edit_file: 'file_write',
    list_dir: 'list_dir',
    read_terminal: 'terminal_read',
    read_notes: 'notes_read',
    update_notes: 'notes_write',
    agent_tool: 'tool_call',
  };
  const operationKinds = new Set([...Object.values(recordKinds), 'approval_asked']);
  const stateIcons: Record<ReturnType<typeof outcomeState>, IconName> = {
    running: 'more', done: 'check', failed: 'warning', denied: 'close',
  };

  let { initial = null, role, onclose, onresume }: Props = $props();
  // svelte-ignore state_referenced_locally
  let filter = $state<(typeof filters)[number]>(role ?? 'all');
  // svelte-ignore state_referenced_locally
  let compactDetail = $state(initialCompactDetail(!!initial));
  let view = $state<(typeof views)[number]>('conversation');
  let query = $state('');
  let searchInput = $state<HTMLInputElement>();
  let sessions = $state<AiSessionSummary[]>([]);
  let selected = $state<string | null>(null);
  let records = $state<AiRecord[]>([]);
  let renaming = $state<string | null>(null);
  let expanded = $state<Record<string, boolean>>({});
  let detailCache = $state<Record<string, DetailEntry>>({});
  let loadRevision = 0;
  let listLoading = $state(true);
  let listError = $state('');
  let loading = $state(false);
  let error = $state('');

  const filteredSessions = $derived(filterSessions(sessions, query));
  const groups = $derived(groupSessionsByDate(filteredSessions));
  const parent = $derived(sessions.find((s) => s.id === selected || s.children.some((c) => c.id === selected)) ?? null);
  const layoutMode = $derived(masterDetailView(viewport.compact, compactDetail && !!parent));
  const current = $derived(parent?.children.find((s) => s.id === selected) ?? parent);
  const chatRows = $derived(rows(itemsFromRecords(records, { asSubagent: current?.role === 'subagent' })));
  const lines = $derived(records.flatMap((row) => {
    const line = auditLine(row);
    if (!line) return [];
    const ref = row.payload.body_ref;
    const reference = typeof ref === 'string' && ref ? ref : undefined;
    const inline = operationKinds.has(line.kind)
      ? displayDetail(line.kind, row.payload)
      : { kind: 'text' as const, text: line.detail };
    return [{ ts: row.ts, line, reference, inline }];
  }));

  $effect(() => {
    let active = true;
    listLoading = true;
    listError = '';
    sessions = [];
    void load(null);
    aiSessions(filter === 'all' ? null : filter).then((found) => {
      if (!active) return;
      sessions = found;
      const requested = found.some((entry) => entry.id === initial || entry.children.some((child) => child.id === initial))
        ? initial
        : null;
      compactDetail = initialCompactDetail(!!requested);
      const first = requested ?? found[0]?.id;
      if (first) void load(first);
    }).catch((caught: unknown) => {
      if (active) listError = errorText(caught);
    }).finally(() => {
      if (active) listLoading = false;
    });
    return () => {
      active = false;
      ++loadRevision;
    };
  });

  $effect(() => {
    if (layoutMode === 'detail') return pushBackHandler(back);
  });

  function back(): void {
    if (layoutMode === 'detail') compactDetail = false;
    else onclose();
  }

  function selectSession(id: string): void {
    if (selected !== id) void load(id);
    compactDetail = true;
  }

  async function load(id: string | null): Promise<void> {
    const revision = ++loadRevision;
    selected = id;
    renaming = null;
    error = '';
    records = [];
    expanded = {};
    detailCache = {};
    loading = id !== null;
    if (!id) return;
    try {
      const loaded = await aiRecords(id);
      if (revision === loadRevision) records = loaded;
    } catch (caught) {
      if (revision === loadRevision) error = errorText(caught);
    } finally {
      if (revision === loadRevision) loading = false;
    }
  }

  async function loadDetail(reference: string, kind: string): Promise<void> {
    const cached = detailCache[reference];
    if (!selected || (cached && !('error' in cached))) return;
    const revision = loadRevision;
    detailCache[reference] = { loading: true };
    try {
      const payload = await aiAuditDetail(selected, reference);
      if (revision !== loadRevision) return;
      const actualKind = payload && typeof payload === 'object' && 'asked' in payload ? 'approval_asked' : kind;
      detailCache[reference] = { detail: displayDetail(actualKind, payload) };
    } catch (caught) {
      if (revision === loadRevision) detailCache[reference] = { error: errorText(caught) };
    }
  }

  function toggle(key: string, kind: string, reference?: string): void {
    const open = !expanded[key];
    expanded[key] = open;
    if (open && reference) void loadDetail(reference, kind);
  }

  function sessionTitle(session: AiSessionSummary): string {
    return session.title ?? (shortHostName(session.host) || t(session.role === 'fleet' ? 'fleet.title' : 'ai.newChatTitle'));
  }

  function sessionMeta(session: AiSessionSummary, title: string): string {
    const host = shortHostName(session.host);
    return [
      filter === 'all' && t(`ai.role.${session.role}`),
      session.role === 'assistant' && host !== title && host,
      session.role === 'fleet' && session.children.length > 0 && t('ai.history.subagentCount', { n: session.children.length }),
      session.model !== session.role && session.model !== 'builtin-assistant' && session.model,
    ].filter(Boolean).join(' · ');
  }

  function clearSearch(): void {
    query = '';
    searchInput?.focus();
  }

  function startRename(): void {
    if (parent) renaming = parent.title ?? '';
  }

  function focusSelect(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }

  function onRenameKey(event: KeyboardEvent): void {
    if (event.isComposing) {
      if (event.key === 'Escape') event.stopPropagation();
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      void rename();
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      renaming = null;
    }
  }

  async function rename(): Promise<void> {
    const session = parent;
    const title = renaming?.trim() ?? '';
    renaming = null;
    if (!session || !title || title === session.title) return;
    try {
      const saved = await ai.renameSession(session.id, title);
      const entry = sessions.find((entry) => entry.id === session.id);
      if (entry) entry.title = saved;
    } catch (caught) {
      if (parent?.id === session.id) error = errorText(caught);
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

  async function remove(): Promise<void> {
    const session = parent;
    if (!session) return;
    const name = sessionTitle(session);
    const confirmed = await ask({ title: t('ai.deleteTitle'), message: t('ai.deleteMessage', { name }), confirmLabel: t('ai.delete'), danger: true });
    if (!confirmed) return;
    try {
      await ai.delete(session.id);
    } catch (caught) {
      if (parent?.id === session.id) error = errorText(caught);
      return;
    }
    const at = sessions.findIndex((entry) => entry.id === session.id);
    const wasSelected = parent?.id === session.id;
    sessions = sessions.filter((entry) => entry.id !== session.id);
    if (!wasSelected) return;
    if (!sessions.length || viewport.compact) {
      compactDetail = false;
      void load(null);
    } else {
      void load(sessions[Math.min(at, sessions.length - 1)]?.id ?? null);
    }
  }

  function time(ts: number): string {
    return new Date(ts).toLocaleString();
  }

  const eventClock = new Intl.DateTimeFormat(undefined, {
    hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23',
  });

  function localized(key: MessageKey, fallback: string): string {
    const text = t(key);
    return text === key ? fallback : text;
  }

  function noteText(line: AuditLine): string {
    if (line.kind === 'command' && (line.exitCode != null || line.note === 'exit –')) {
      return t('ai.audit.exitCode', { code: line.exitCode ?? '–' });
    }
    if (line.kind === 'reviewed' && line.reviewer) {
      const reviewer = localized(`ai.reviewer.${line.reviewer}` as MessageKey, line.reviewer);
      return line.confidence == null ? reviewer : `${reviewer} ${line.confidence}%`;
    }
    const prefix = line.kind === 'approval_asked' ? 'ai.risk.' : line.kind === 'approval_answered' ? 'ai.answer.' : 'ai.result.';
    return localized(`${prefix}${line.note}` as MessageKey, line.note);
  }

  function displayDetail(kind: string, payload: unknown): FormattedAuditDetail {
    const formatted = formatAuditDetail(kind, payload);
    // Metadata-only operation records have no expandable user-facing output.
    if (formatted.kind !== 'json' || !operationKinds.has(kind)) return formatted;
    if (kind === 'tool_call') {
      if (typeof payload === 'string') return formatted;
      if (payload && typeof payload === 'object') {
        const { input, outcome } = payload as { input?: unknown; outcome?: { content?: unknown } };
        if (input != null || typeof outcome?.content === 'string' && outcome.content.trim()) return formatted;
      }
    }
    return { kind: 'text', text: typeof payload === 'string' ? payload : '' };
  }

  function inlineOperation(item: GroupItem | ApprovalItem, kind: string): FormattedAuditDetail {
    if (item.kind === 'tool') {
      const content = item.call.content.map((part) => part.type === 'text' ? part.text : '').filter(Boolean).join('\n');
      if (!content && item.call.raw_input == null) return { kind: 'text', text: '' };
      return formatAuditDetail(kind, { input: item.call.raw_input, outcome: { content } });
    }
    const { operation, outcome } = item;
    if (item.kind === 'approval' && !outcome) return displayDetail(kind, { asked: operation });
    return displayDetail(kind, { operation, outcome: outcome?.result === 'command' ? outcome.output : outcome });
  }
</script>

{#snippet formattedDetail(inline: FormattedAuditDetail, reference?: string)}
  {@const cached = reference ? detailCache[reference] : undefined}
  {@const detail = cached && 'detail' in cached ? cached.detail : inline}
  {#if reference && (!cached || 'loading' in cached)}
    <p class="muted detail-status" role="status">{t('ai.history.loadingDetail')}</p>
  {:else}
    {#if cached && 'error' in cached}<p class="error detail-status">{cached.error}</p>{/if}
    {#if detail.text}
      {#if detail.kind === 'diff'}
        <pre class="diff selectable">{#each detail.text.split('\n') as line, at (at)}{at ? '\n' : ''}<span class:added={line.startsWith('+ ')} class:removed={line.startsWith('- ')}>{line}</span>{/each}</pre>
      {:else}
        <pre class="op-output selectable">{detail.text.includes('could not confirm that the remote command stopped') ? t('ai.remoteExitUnknown') : detail.text}</pre>
      {/if}
    {:else if !(cached && 'error' in cached)}
      <p class="muted detail-status">{t('ai.noOutput')}</p>
    {/if}
  {/if}
{/snippet}

{#snippet operationRow(item: GroupItem | ApprovalItem, key: string)}
  {@const tool = item.kind === 'tool' ? item.call : null}
  {@const operation = item.kind !== 'tool' ? item.operation : null}
  {@const toolName = tool?.title ?? (operation?.op === 'agent_tool' ? operation.title : null)}
  {@const leadTool = isLeadTool(toolName) ? toolName : null}
  {@const icon = leadTool ? LEAD_TOOL_ICONS[leadTool] : operation ? OPERATION_ICONS[operation.op] : 'command'}
  {@const verb = leadTool ? t(`ai.tool.${leadTool}`) : operation ? t(`ai.verb.${operation.op}`) : tool?.title ?? tool?.kind ?? tool?.id}
  {@const what = tool ? toolSubject(tool) : operation ? subject(operation) : ''}
  {@const kind = item.kind === 'approval' && !item.outcome ? 'approval_asked' : operation ? recordKinds[operation.op] : 'tool_call'}
  {@const state = item.kind === 'tool'
    ? item.call.status === 'completed' ? 'done' : item.call.status === 'failed' ? 'failed' : 'running'
    : item.kind === 'approval' && (item.answer === 'deny' || item.answer === null) && !item.outcome ? 'denied' : outcomeState(item.outcome)}
  {@const detail = inlineOperation(item, kind)}
  <button type="button" class="op {state}" aria-expanded={!!expanded[key]} disabled={!item.bodyRef && !detail.text} onclick={() => toggle(key, kind, item.bodyRef)}>
    <span class="op-icon"><Icon name={icon} size={13} /></span>
    <span class="op-verb" title={verb}>{verb}</span>
    {#if what}<code class="op-subject" title={what}>{what}</code>{/if}
    {#if item.kind === 'operation' && item.reviewer}
      <span class="tag" title={t('ai.reviewedBy', { reviewer: localized(`ai.reviewer.${item.reviewer}` as MessageKey, item.reviewer) })}>{t('ai.reviewed')}</span>
    {/if}
    {#if item.kind === 'approval' && item.answer !== undefined}
      <span class="tag">{t(`ai.answer.${item.answer ?? 'withdrawn'}`)}</span>
    {/if}
    <span class="op-state" title={t(`ai.state.${state}`)}><Icon name={stateIcons[state]} size={12} /></span>
  </button>
  {#if expanded[key]}{@render formattedDetail(detail, item.bodyRef)}{/if}
{/snippet}

{#snippet footer()}
  <button type="button" class="primary" onclick={onclose}>{t('settings.done')}</button>
{/snippet}

<Dialog presentation="page" title={t('ai.history')} onclose={back} width={960} actions={viewport.compact ? undefined : footer}>
  <div class="audit" class:compact={viewport.compact}>
    {#if layoutMode !== 'detail'}
    <nav aria-label={t('ai.history')}>
      <div class="search-box">
        <Icon name="search" size={13} />
        <input class="bare" type="search" bind:this={searchInput} bind:value={query} placeholder={t('ai.historySearch')} aria-label={t('ai.historySearch')} />
        {#if query}<IconButton icon="close" small label={t('ai.historyClearSearch')} onclick={clearSearch} />{/if}
      </div>
      <div class="segmented" role="tablist" aria-label={t('ai.history')}>
        {#each filters as kind (kind)}
          <button type="button" role="tab" aria-selected={filter === kind} onclick={() => (filter = kind)}>{t(`ai.role.${kind}`)}</button>
        {/each}
      </div>
      <div class="session-list">
        {#if layoutMode === 'list' && listError}
          <p class="error" role="alert">{listError}</p>
        {:else if layoutMode === 'list' && listLoading}
          <p class="empty" role="status">{t('ai.historyLoading')}</p>
        {:else if !listLoading}
          {#if sessions.length === 0}
            <p class="empty">{t('ai.historyEmpty')}</p>
          {:else if filteredSessions.length === 0}
            <p class="empty">{t('ai.historyNoMatch')}</p>
          {/if}
          {#each groups as group (group.bucket)}
            <div class="session-group">
              <h3 class="section-label">{t(`ai.history.${group.bucket}`)}</h3>
              {#each group.sessions as session (session.id)}
                {@const title = sessionTitle(session)}
                {@const meta = sessionMeta(session, title)}
                <button type="button" class="session" class:active={session.id === parent?.id} aria-current={session.id === parent?.id ? 'true' : undefined} onclick={() => selectSession(session.id)}>
                  <span class="session-top">
                    <span class="session-title" title={title}>{title}</span>
                    <span class="session-time" title={time(session.started)}>{formatSessionTime(session.started)}</span>
                  </span>
                  {#if meta}<span class="session-meta" title={meta}>{meta}</span>{/if}
                </button>
              {/each}
            </div>
          {/each}
        {/if}
      </div>
    </nav>
    {/if}

    {#if layoutMode !== 'list'}
    <section class="pane" aria-label={t('ai.history')}>
      {#if parent}
        <header class="pane-head">
          <div class="head-info">
            <div class="head-title-row">
              {#if renaming !== null}
                <input class="rename-field" use:focusSelect bind:value={renaming} maxlength="120" aria-label={t('ai.rename')} onkeydown={onRenameKey} onblur={() => void rename()} />
              {:else}
                <button type="button" class="head-title" title={t('ai.renameHint')} onclick={startRename}>{sessionTitle(parent)}</button>
                <IconButton icon="edit" small label={t('ai.rename')} onclick={startRename} />
              {/if}
            </div>
            <div class="head-meta">
              <span>{t(`ai.role.${parent.role}`)}</span>
              {#if parent.role === 'assistant' && parent.host}<span title={parent.host}>{shortHostName(parent.host)}</span>{/if}
              <span>{time(parent.started)}</span>
              {#if parent.model ?? parent.agent}<span>{parent.model ?? parent.agent}</span>{/if}
            </div>
          </div>
          <div class="head-actions">
            <div class="segmented" role="tablist" aria-label={t('ai.history')}>
              {#each views as kind (kind)}
                <button type="button" role="tab" aria-selected={view === kind} onclick={() => (view = kind)}>{t(kind === 'conversation' ? 'ai.history.viewConversation' : 'ai.history.viewAudit')}</button>
              {/each}
            </div>
            <IconButton icon="remove" small label={t('ai.delete')} onclick={() => void remove()} />
            {#if onresume}
              <button type="button" class="primary" disabled={!parent.agent_ref || !parent.context || (current === parent && !records.length)} title={parent.agent_ref ? '' : t('ai.resumeUnavailable')} onclick={resumeParent}>{t('ai.resume')}</button>
            {/if}
          </div>
        </header>
        {#if parent.children.length > 0}
          <div class="switcher" role="group" aria-label={t('ai.subagents.auditViews')}>
            <button type="button" aria-pressed={current === parent} onclick={() => parent && current !== parent && void load(parent.id)}>{t('ai.subagents.lead')}</button>
            {#each parent.children as child (child.id)}
              <button type="button" aria-pressed={current === child} title={child.host ? `${child.host} · ${time(child.started)}` : time(child.started)} onclick={() => current !== child && void load(child.id)}>
                <Icon name="server" size={12} />
                {child.host ? shortHostName(child.host) : t('ai.role.subagent')}
              </button>
            {/each}
          </div>
        {/if}
      {/if}
      {#key selected}
        <div class="content">
          {#if listError || error}<p class="error" role="alert">{listError || error}</p>{/if}
          {#if loading || listLoading}
            <p class="empty" role="status">{t('ai.historyLoading')}</p>
          {:else if !current}
            {#if !listError && !error}<p class="empty">{t('ai.historySelect')}</p>{/if}
          {:else if view === 'conversation'}
            <div class="conversation">
              {#each chatRows as row (row.key)}
                {#if row.kind === 'group'}
                  {@const key = `g${row.key}`}
                  <div>
                    <button type="button" class="op group done" aria-expanded={!!expanded[key]} onclick={() => (expanded[key] = !expanded[key])}>
                      <span class="op-icon"><Icon name={expanded[key] ? 'chevronDown' : 'chevron'} size={12} /></span>
                      <span class="op-verb">{t('ai.ranGroup', { n: row.items.length })}</span>
                      <span class="op-state"><Icon name="check" size={12} /></span>
                    </button>
                    {#if expanded[key]}
                      <div class="group-items">
                        {#each row.items as item, at (at)}
                          <div>{@render operationRow(item, `${key}-${at}`)}</div>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {:else}
                  {@const item = row.item}
                  {#if item.kind === 'user'}
                    <div class="msg-user selectable">{item.text}</div>
                  {:else if item.kind === 'delegation'}
                    <div class="msg-delegation">
                      <strong>{t('ai.subagents.delegatedTask')}</strong>
                      <p class="selectable">{item.task}</p>
                    </div>
                  {:else if item.kind === 'agent'}
                    <div class="msg-agent"><Markdown text={item.text} /></div>
                  {:else if item.kind === 'thought' || item.kind === 'compacted'}
                    <details class="thought">
                      <summary>{t(item.kind === 'thought' ? 'ai.thought' : 'ai.compacted')}</summary>
                      <pre class="selectable">{item.kind === 'thought' ? item.text : item.summary}</pre>
                    </details>
                  {:else if item.kind === 'tool' || item.kind === 'operation' || item.kind === 'approval'}
                    <div>{@render operationRow(item, `i${row.key}`)}</div>
                  {:else if item.kind === 'connection'}
                    <details class="notice" class:warning={item.state !== 'recovered'}>
                      <summary>{item.state === 'retrying' ? t('ai.connection.retrying', { n: item.retries }) : t(`ai.connection.${item.state}`)}</summary>
                      <pre class="selectable">{item.detail}</pre>
                    </details>
                  {:else if item.kind === 'notice'}
                    <p class="notice warning selectable">{item.text}</p>
                  {/if}
                {/if}
              {/each}
            </div>
          {:else}
            {#each lines as { ts, line, reference, inline }}
              {@const leadTool = line.kind === 'tool_call' && isLeadTool(line.text) ? line.text : null}
              {@const note = noteText(line)}
              <div class="line {line.tone}">
                <time class="time">{eventClock.format(ts)}</time>
                <span class="kind">{current.role === 'subagent' && line.kind === 'prompt' ? t('ai.subagents.delegatedTask') : localized(`ai.audit.${line.kind}` as MessageKey, line.kind)}</span>
                <div class="body">
                  {#if line.text}
                    <span class="text selectable" class:mono={line.kind === 'command'}>{leadTool ? t(`ai.tool.${leadTool}`) : line.text}</span>
                  {/if}
                  {#if line.toolSubject}<code class="text selectable">{line.toolSubject}</code>{/if}
                  {#if note}<span class="note">{note}</span>{/if}
                  {#if line.kind === 'message' && line.detail}
                    <details>
                      <summary>{t('ai.historyDetail')}</summary>
                      <div class="audit-message"><Markdown text={line.detail} /></div>
                    </details>
                  {:else if reference || inline.text}
                    <details ontoggle={(event) => { if (reference && event.currentTarget.open) void loadDetail(reference, line.kind); }}>
                      <summary>{t('ai.historyDetail')}</summary>
                      {@render formattedDetail(inline, reference)}
                    </details>
                  {/if}
                </div>
              </div>
            {/each}
          {/if}
        </div>
      {/key}
    </section>
    {/if}
  </div>
</Dialog>

<style>
  .audit {
    height: min(660px, 74vh);
    display: flex;
    margin: 0 calc(-1 * var(--space-5));
    border-block: 1px solid var(--separator);
    background: var(--surface);
  }
  nav {
    flex: 0 0 264px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-2) 0;
    border-inline-end: 1px solid var(--separator);
    background: var(--material-sidebar);
  }
  .search-box {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-height);
    padding-inline: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .search-box:focus-within { box-shadow: var(--focus-ring); }
  .search-box input {
    width: 100%;
    min-width: 0;
    font-size: var(--text-footnote);
  }
  .search-box input::-webkit-search-cancel-button { display: none; }
  nav .segmented {
    flex: none;
    width: 100%;
  }
  nav .segmented > button {
    flex: 1 1 0;
    min-width: 0;
    height: auto;
    min-height: 22px;
    padding: var(--space-1);
    line-height: 1.15;
    overflow-wrap: anywhere;
  }
  .session-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-block: var(--space-1) var(--space-2);
  }
  .session-group { margin-bottom: var(--space-3); }
  .section-label {
    margin-bottom: var(--space-1);
    text-transform: uppercase;
  }
  .session {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 100%;
    margin-bottom: 2px;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .session:hover { background: var(--fill-hover); }
  .session.active { background: var(--accent-soft); }
  .session-top {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .session-title { font-weight: 600; }
  .session-title, .session-meta, .head-title, .op-verb, .op-subject {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .session-time, .session-meta {
    color: var(--fg-muted);
    font-size: var(--text-caption);
  }
  .session-time, .time {
    flex: none;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .pane {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .pane-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2) var(--space-4);
    flex: none;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
    background: var(--material-bar);
  }
  .head-info { flex: 1 1 180px; min-width: 0; }
  .head-title-row, .head-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .head-actions { flex-wrap: wrap; }
  .head-title {
    padding: 0;
    border: none;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-weight: 600;
    font-size: var(--text-title);
    text-align: start;
    cursor: text;
  }
  .head-title:hover { color: var(--accent); }
  .rename-field {
    width: 100%;
    min-width: 0;
    min-height: 24px;
    padding-block: 1px;
    font-size: var(--text-title);
  }
  .head-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 2px var(--space-2);
    margin-top: var(--space-1);
    color: var(--fg-muted);
    font-size: var(--text-caption);
    overflow-wrap: anywhere;
  }
  .head-meta > span + span::before { content: '·'; margin-inline-end: var(--space-2); }
  .switcher {
    display: flex;
    flex: none;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    overflow-x: auto;
    border-bottom: 1px solid var(--separator);
  }
  .switcher button {
    display: flex;
    align-items: center;
    flex: none;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-pill);
    background: var(--fill);
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    white-space: nowrap;
    cursor: pointer;
  }
  .switcher button:hover { color: var(--fg); }
  .switcher button[aria-pressed='true'] {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-4);
  }
  .empty, .detail-status { margin: var(--space-2); font-size: var(--text-footnote); }
  .empty, .muted, .thought, .notice { color: var(--fg-muted); }
  .conversation {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .msg-user, .msg-delegation {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .msg-user {
    align-self: flex-end;
    max-width: 88%;
    border-end-end-radius: var(--radius-xs);
    background: var(--accent-soft);
  }
  .msg-delegation {
    border-inline-start: 2px solid var(--accent);
    background: var(--accent-softer);
  }
  .msg-delegation strong { color: var(--fg-muted); font-size: var(--text-caption); }
  .msg-delegation p { margin: var(--space-1) 0 0; }
  .msg-agent { padding-block: var(--space-1); min-width: 0; }
  .thought, .notice { margin: 0; font-size: var(--text-footnote); overflow-wrap: anywhere; }
  .notice.warning { color: var(--warning); }
  details > summary { cursor: pointer; color: var(--fg-muted); }
  .warning > summary { color: var(--warning); }
  .op {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: 28px;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-footnote);
    text-align: start;
    cursor: pointer;
  }
  .op:hover:not(:disabled), .op[aria-expanded='true'] { background: var(--fill-hover); }
  .op:disabled { cursor: default; }
  .op-icon, .op-state { display: grid; place-items: center; flex: none; color: var(--fg-muted); }
  .op-state { margin-inline-start: auto; }
  .done .op-state { color: var(--success); }
  .failed .op-state { color: var(--warning); }
  .denied .op-state { color: var(--danger); }
  .op-verb { flex: 0 1 auto; font-weight: 600; }
  .op-subject { flex: 1 1 0; color: var(--fg-muted); }
  .tag {
    padding: 1px var(--space-1);
    border-radius: var(--radius-xs);
    background: var(--fill-pressed);
    font-size: var(--text-caption);
  }
  .group-items {
    display: grid;
    gap: var(--space-1);
    margin: var(--space-1) 0 0 var(--space-2);
    padding-inline-start: var(--space-2);
    border-inline-start: 1px solid var(--separator);
  }
  .line {
    display: grid;
    grid-template-columns: max-content minmax(64px, 98px) minmax(0, 1fr);
    gap: var(--space-2);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--separator);
    font-size: var(--text-footnote);
  }
  .time, .note { color: var(--fg-muted); }
  .kind { font-weight: 600; }
  .line.warning .kind, .line.warning .note { color: var(--warning); }
  .line.danger .kind, .line.danger .note { color: var(--danger); }
  .body { display: flex; flex-direction: column; gap: var(--space-1); min-width: 0; }
  .text { white-space: pre-wrap; overflow-wrap: anywhere; }
  code, .mono, pre { font-family: var(--font-mono); }
  pre, .audit-message {
    margin: var(--space-1) 0 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
  }
  pre {
    max-height: 220px;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--text-footnote);
  }
  .diff .added { color: var(--success); }
  .diff .removed { color: var(--danger); }
  @media (max-width: 720px) {
    nav { flex-basis: clamp(112px, 34vw, 204px); }
    .session-top { flex-wrap: wrap; gap: 1px var(--space-1); }
    .pane-head, .switcher, .content { padding-inline: var(--space-2); }
    .head-meta > span + span::before { display: none; }
    .line { grid-template-columns: max-content minmax(0, 1fr); }
    .line .body { grid-column: 2; }
  }
  .audit.compact {
    flex: 1;
    height: auto;
    min-height: 0;
    margin: calc(-1 * var(--space-3)) calc(-1 * var(--space-4));
    border-block: none;
  }
  .compact nav {
    flex: 1;
    border-inline-end: none;
  }
  .compact .session {
    min-height: 48px;
    justify-content: center;
  }
  .compact .segmented > button {
    min-height: 36px;
    height: auto;
  }
  .compact .switcher button { min-height: 36px; }
  .compact .op { min-height: 44px; }
</style>
