<script lang="ts">
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Markdown from '../ai/Markdown.svelte';
  import { ask } from '../ui/confirm.svelte';
  import { t } from '../i18n';
  import { ops } from './ops.svelte';
  import { canCancelTask, canRetrySubtask, canRetryTask, isSubtaskActive, uncertainHosts } from './taskview';
  import type { OpsSubtaskView, OpsTaskCounts, OpsTaskState, OpsTaskView } from './types';

  let { conversation, onselectsubagent }: { conversation: string; onselectsubagent?: (sessionId: string, hostName: string) => void } = $props();
  const tasks = $derived(ops.tasksOf(conversation));
  const primary = $derived(tasks[0]);
  const expandedByConversation = new SvelteMap<string, boolean>();
  const expanded = $derived(expandedByConversation.get(conversation) ?? tasks.some(canCancelTask));
  const pending = new SvelteSet<string>();
  const failedActions = new SvelteSet<string>();
  const error = $derived(failedActions.has(conversation) ? t('error.ops.failed') : ops.errorOf(conversation));
  let report = $state<{ taskId: string; hostId?: string } | null>(null);
  const reportTask = $derived(tasks.find((task) => task.id === report?.taskId));
  const reportHost = $derived(reportTask?.subtasks.find((sub) => sub.host_id === report?.hostId));
  const waitingHost = $derived(
    primary?.subtasks.find((sub) => sub.state === 'waiting_approval' && sub.session)
    ?? tasks.flatMap((task) => task.subtasks).find((sub) => sub.state === 'waiting_approval' && sub.session),
  );
  const clock = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });

  $effect(() => {
    void ops.load(conversation);
    report = null;
  });

  function headerState(task: OpsTaskView): OpsTaskState {
    const counts = task.counts;
    if (counts.waiting_approval) return 'waiting_approval';
    if (counts.running || counts.queued) return 'running';
    return task.state;
  }

  function progress(counts: OpsTaskCounts): string {
    const amounts = { done: counts.completed, total: counts.total };
    if (counts.total > 0 && counts.completed === counts.total) return t('ops.allDone', amounts);
    const parts = [t('ops.progress', amounts)];
    const active = counts.running + counts.queued;
    const blocked = counts.blocked + counts.interrupted;
    if (active) parts.push(t('ops.runningCount', { n: active }));
    if (counts.waiting_approval) parts.push(t('ops.waitingCount', { n: counts.waiting_approval }));
    if (counts.failed) parts.push(t('ops.failedCount', { n: counts.failed }));
    if (blocked) parts.push(t('ops.blockedCount', { n: blocked }));
    if (counts.cancelled) parts.push(t('ops.stoppedCount', { n: counts.cancelled }));
    return parts.join(' · ');
  }

  function hostStatus(sub: OpsSubtaskView): string {
    switch (sub.state) {
      case 'completed': return sub.summary ?? t('ops.state.completed');
      case 'failed': return sub.error ?? t('ops.state.failed');
      case 'blocked':
      case 'interrupted': return sub.error ?? (sub.reason ? t(`ops.reason.${sub.reason}`) : t(`ops.state.${sub.state}`));
      default: return t(`ops.state.${sub.state}`);
    }
  }

  function hasDetails(sub: OpsSubtaskView): boolean {
    return !!(sub.reply || sub.summary || sub.error || sub.reason || sub.checks.length);
  }

  function openLog(sub: OpsSubtaskView): void {
    if (sub.session) onselectsubagent?.(sub.session, sub.host_name);
    report = null;
  }

  async function perform(task: OpsTaskView, action: () => Promise<unknown>): Promise<void> {
    if (pending.has(task.id)) return;
    pending.add(task.id);
    failedActions.delete(task.conversation);
    try {
      await action();
    } catch {
      failedActions.add(task.conversation);
    } finally {
      pending.delete(task.id);
    }
  }

  function retry(task: OpsTaskView, hostId: string | null = null): Promise<void> {
    return perform(task, async () => {
      const names = uncertainHosts(task, hostId);
      const confirmed = names.length > 0;
      if (confirmed && !(await ask({
        title: t('ops.confirmTitle'),
        message: t('ops.confirmMessage', { hosts: names.join(', ') }),
        confirmLabel: t('ops.confirmContinue'),
        danger: true,
      }))) return;
      if (task.conversation === conversation) await ops.retryTask(task.id, hostId, confirmed);
    });
  }
</script>

{#if primary || error}
  <section class="tasks" aria-label={t('ops.title')}>
    {#if primary}
      {@const counts = primary.counts}
      {@const summary = progress(counts)}
      <header class="summary">
        <button type="button" class="toggle" aria-expanded={expanded} aria-label={t(expanded ? 'ops.collapse' : 'ops.expand')} onclick={() => expandedByConversation.set(conversation, !expanded)}>
          <span class="dot {headerState(primary)}" aria-hidden="true"></span>
          <strong class="ellipsis title" title={primary.title}>{primary.title}</strong>
          {#if tasks.length > 1}<span class="tag task-count">{t('ops.count', { n: tasks.length })}</span>{/if}
          <span class="progress ellipsis" title={summary}>{summary}</span>
          {#if counts.total > 0}
            <span class="track" aria-hidden="true">
              <span class="done" style:width={`${100 * counts.completed / counts.total}%`}></span>
              <span class="active-segment" style:width={`${100 * (counts.queued + counts.running + counts.waiting_approval) / counts.total}%`}></span>
            </span>
          {/if}
          <span class="chevron" class:expanded><Icon name="chevronDown" size={13} /></span>
        </button>
        {#if waitingHost && onselectsubagent}
          <button type="button" class="plain action review" onclick={(event) => { event.stopPropagation(); openLog(waitingHost); }}>{t('ops.goApprove')}</button>
        {:else if canCancelTask(primary)}
          <button type="button" class="plain action" disabled={pending.has(primary.id)} onclick={(event) => { event.stopPropagation(); void perform(primary, () => ops.cancelTask(primary.id)); }}>{t('ops.stop')}</button>
        {:else if canRetryTask(primary)}
          <button type="button" class="plain action" disabled={pending.has(primary.id)} onclick={(event) => { event.stopPropagation(); void retry(primary); }}>{t('ops.retry')}</button>
        {/if}
      </header>
    {/if}
    {#if error}<p class="load-error" role="alert">{error}</p>{/if}
    {#if expanded && primary}
      <div class="drawer">
        {#each tasks as task (task.id)}
          <div class="task">
            <div class="meta">
              {#if tasks.length > 1}<strong class="ellipsis" title={task.title}>{task.title}</strong>{/if}
              <time>{clock.format(task.updated_at)}</time>
              {#if task.source.kind === 'schedule'}<span class="tag">{t('ops.scheduleTag')}</span>{/if}
              {#if task.read_only}<span class="tag">{t('ops.readOnlyTag')}</span>{/if}
              <div class="actions">
                {#if task.subtasks.some(hasDetails)}
                  <button type="button" class="plain action" onclick={() => (report = { taskId: task.id })}>{t('ops.viewReport')}</button>
                {/if}
                {#if canRetryTask(task)}
                  <button type="button" class="plain action" disabled={pending.has(task.id)} onclick={() => void retry(task)}>{t('ops.retryFailed')}</button>
                {/if}
                {#if canCancelTask(task)}
                  <button type="button" class="plain action" disabled={pending.has(task.id)} onclick={() => void perform(task, () => ops.cancelTask(task.id))}>{t('ops.stop')}</button>
                {/if}
              </div>
            </div>
            {#each task.subtasks as sub (sub.host_id)}
              {@const status = hostStatus(sub)}
              {@const subtaskId = sub.id}
              <div class="host-row">
                <span class="dot {sub.state}" aria-hidden="true"></span>
                <strong class="host ellipsis" title={sub.host_name}>{sub.host_name}</strong>
                <span class="host-status ellipsis {sub.state}" title={status}>{status}</span>
                <div class="actions">
                  {#if sub.session && onselectsubagent}
                    {@const awaiting = sub.state === 'waiting_approval'}
                    <button type="button" class="plain action" class:review={awaiting} onclick={() => openLog(sub)}>{t(awaiting ? 'ops.goApprove' : 'ops.viewProcess')}</button>
                  {/if}
                  {#if hasDetails(sub)}
                    <button type="button" class="plain action" onclick={() => (report = { taskId: task.id, hostId: sub.host_id })}>{t('ops.viewDetail')}</button>
                  {/if}
                  {#if isSubtaskActive(sub) && subtaskId}
                    <IconButton icon="stop" small label={t('ops.stopHost', { host: sub.host_name })} disabled={pending.has(task.id)} onclick={() => void perform(task, () => ops.cancelSubtask(task.id, subtaskId))} />
                  {/if}
                  {#if canRetrySubtask(sub)}
                    <IconButton icon="refresh" small label={t('ops.retryHost', { host: sub.host_name })} disabled={pending.has(task.id)} onclick={() => void retry(task, sub.host_id)} />
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        {/each}
      </div>
    {/if}
  </section>
{/if}

{#snippet result(sub: OpsSubtaskView, single: boolean)}
  <section class="result">
    <header class="result-header">
      {#if !single}<strong>{sub.host_name}</strong>{/if}
      <span class="state-pill {sub.state}">{t(`ops.state.${sub.state}`)}</span>
    </header>
    {#if single && sub.task.trim()}
      <blockquote class="task-prompt selectable"><strong>{t('ops.taskPrompt')}</strong><p>{sub.task}</p></blockquote>
    {/if}
    {#if sub.error || sub.reason}
      <p class="result-alert {sub.state} selectable">{sub.error ?? (sub.reason && t(`ops.reason.${sub.reason}`))}</p>
    {/if}
    {#if sub.checks.length}
      <div class="checks">
        <strong>{t('ops.checks')}</strong>
        <ul>
          {#each sub.checks as check}
            <li>{check.criterion}{#if check.summary}<span>{check.summary}</span>{/if}</li>
          {/each}
        </ul>
      </div>
    {/if}
    {#if sub.reply}
      <Markdown text={sub.reply} />
    {:else if sub.summary}
      <p class="selectable">{sub.summary}</p>
    {:else if !sub.error && !sub.reason && !sub.checks.length}
      <p class="muted">{t('ops.noReportYet')}</p>
    {/if}
  </section>
{/snippet}

{#if reportTask}
  <Dialog title={reportHost ? t('ops.hostResultTitle', { host: reportHost.host_name }) : t('ops.reportTitle')} width={580} onclose={() => (report = null)}>
    {#if reportHost}
      {@render result(reportHost, true)}
    {:else}
      <div class="report-heading">
        <strong>{reportTask.title}</strong>
        <span class="progress">{progress(reportTask.counts)}</span>
      </div>
      {#each reportTask.subtasks as sub (sub.host_id)}
        {@render result(sub, false)}
      {/each}
    {/if}
    {#snippet actions()}
      {#if reportHost?.session && onselectsubagent}
        <button type="button" class="secondary" onclick={() => openLog(reportHost)}>{t('ai.subagents.viewLog')}</button>
      {/if}
      <button type="button" class="primary" onclick={() => (report = null)}>{t('common.close')}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .tasks {
    container: ops-card / inline-size;
    flex: none;
    margin: 0 var(--space-3) var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: 0 0 0 0.5px var(--separator), var(--shadow-sm);
  }
  .summary, .toggle, .meta, .host-row, .actions, .result-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .summary { padding-inline: var(--space-2); }
  .toggle {
    flex: 1;
    min-width: 0;
    min-height: 36px;
    border: none;
    border-radius: var(--radius-sm);
    padding: 0 var(--space-1);
    background: transparent;
    color: var(--fg);
    text-align: start;
    font: inherit;
    cursor: pointer;
  }
  .toggle:hover { background: var(--fill); }
  .title { flex: 1; }
  .ellipsis {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .progress {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    font-variant-numeric: tabular-nums;
  }
  .toggle .progress { flex: 0 1 auto; }
  .track {
    display: flex;
    width: 44px;
    height: 4px;
    flex: none;
    overflow: hidden;
    background: var(--fill);
    border-radius: var(--radius-pill);
  }
  .done { background: var(--success); }
  .active-segment { background: var(--accent-soft); }
  .chevron {
    flex: none;
    display: inline-flex;
    color: var(--fg-muted);
    transition: transform var(--dur-fast) var(--ease-out);
  }
  .chevron.expanded { transform: rotate(180deg); }
  .drawer {
    max-height: min(36vh, 320px);
    overflow-y: auto;
    border-top: 1px solid var(--separator);
  }
  .task { padding: var(--space-1) var(--space-3); }
  .task + .task { border-top: 1px solid var(--separator); }
  .meta {
    flex-wrap: wrap;
    min-height: 26px;
    color: var(--fg-muted);
    font-size: var(--text-caption);
  }
  .meta strong { color: var(--fg); }
  time, .tag { white-space: nowrap; }
  .tag {
    padding: 2px var(--space-2);
    border-radius: var(--radius-pill);
    background: var(--fill);
    color: var(--fg-muted);
    font-size: var(--text-caption);
  }
  .host-row { min-height: 28px; }
  .host-row + .host-row { border-top: 0.5px solid var(--separator); }
  .host { max-width: 28%; font-weight: 600; }
  .host-status { flex: 1; color: var(--fg-muted); font-size: var(--text-footnote); }
  .actions { flex: none; gap: var(--space-1); margin-inline-start: auto; }
  .action {
    height: 24px;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .summary > .action { background: var(--fill); }
  .action.review, .summary > .action.review {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .action:hover:not(:disabled) { background: var(--fill-hover); color: var(--fg); }
  .action.review:hover:not(:disabled) { color: var(--warning); }
  .dot {
    width: 6px;
    height: 6px;
    flex: none;
    border-radius: 50%;
    background: var(--status-color, var(--fg-subtle));
  }
  .running, .queued { --status-color: var(--accent); }
  .waiting_approval, .blocked, .interrupted { --status-color: var(--warning); }
  .completed { --status-color: var(--success); }
  .failed { --status-color: var(--danger); }
  .dot.running { animation: pulse 1.4s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.4; } }
  .host-status.waiting_approval, .host-status.blocked, .host-status.interrupted, .host-status.failed {
    color: var(--status-color);
  }
  .load-error {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    color: var(--danger);
    font-size: var(--text-footnote);
  }
  .report-heading, .result { display: grid; gap: var(--space-2); min-width: 0; overflow-wrap: anywhere; }
  .report-heading { font-size: var(--text-title); }
  .report-heading + .result, .result + .result {
    border-top: 1px solid var(--separator);
    padding-top: var(--space-3);
  }
  .state-pill {
    padding: 2px var(--space-2);
    border-radius: var(--radius-pill);
    color: var(--status-color, var(--fg-muted));
    background: var(--fill);
    font-size: var(--text-caption);
  }
  .task-prompt, .result-alert {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
  }
  .task-prompt {
    border-inline-start: 2px solid var(--separator);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .task-prompt p { margin: var(--space-1) 0 0; white-space: pre-wrap; }
  .result-alert {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .result-alert.failed { background: var(--danger-soft); color: var(--danger); }
  .checks ul { margin: var(--space-1) 0 0; padding-inline-start: var(--space-5); }
  .checks li span { display: block; color: var(--fg-muted); }
  .checks li + li { margin-top: var(--space-1); }
  .muted { margin: 0; color: var(--fg-muted); }
  @container ops-card (width < 460px) {
    .host-row {
      flex-wrap: wrap;
      padding-block: var(--space-2);
      row-gap: var(--space-1);
    }
    .host { max-width: 48%; }
    .host-status { min-width: 80px; }
    .host-row .actions {
      width: 100%;
      justify-content: flex-end;
      flex-wrap: wrap;
    }
    .host-row .actions:empty { display: none; }
  }
  @container ops-card (width < 420px) { .track { display: none; } }
  @container ops-card (width < 310px) {
    .task-count { padding-inline: var(--space-1); }
    .toggle { gap: var(--space-1); }
    .host-row { gap: var(--space-1); }
  }
  @media (pointer: coarse) {
    .toggle { min-height: var(--row-height); }
    .action {
      min-height: 34px;
      height: auto;
      padding-inline: var(--space-2);
    }
  }
  @media (prefers-reduced-motion: reduce) { .dot.running { animation: none; } }
</style>
