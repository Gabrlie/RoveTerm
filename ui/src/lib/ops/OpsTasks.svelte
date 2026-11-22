<script lang="ts">
  import { onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import Markdown from '../ai/Markdown.svelte';
  import { ask } from '../ui/confirm.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { errorText, t, type MessageKey } from '../i18n';
  import { opsTaskCancel, opsTaskEvidence, opsTaskInput, opsTaskRetry, opsTasks } from '../ipc';
  import { canAppend, canCancel, canRetry, evidenceText, isUncertain, OpsRefreshGate, reconcileTasks, selectedTargets, selectedUncertain, subtaskFor, targetChoice, type TargetChoices } from './taskview';
  import type { OpsTask, PrivateRef } from './types';

  let { conversation, onselectsubagent }: { conversation: string; onselectsubagent?: (sessionId: string, hostName: string) => void } = $props();
  let tasks = $state<OpsTask[]>([]);
  let refreshing = $state(false);
  let error = $state('');
  let stale = $state(false);
  let inputs = $state<Record<string, string>>({});
  let choices = $state<Record<string, TargetChoices>>({});
  let busy = $state<Record<string, boolean>>({});
  let messages = $state<Record<string, string>>({});
  let gate = new OpsRefreshGate();
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  let evidence = $state<{ title: string; body: unknown; loading: boolean; error: string } | null>(null);
  let evidenceRequest = 0;

  $effect(() => {
    const id = conversation;
    const current = new OpsRefreshGate();
    gate = current;
    tasks = [];
    inputs = {};
    choices = {};
    busy = {};
    messages = {};
    error = '';
    stale = false;
    evidence = null;
    ++evidenceRequest;
    if (id) void refresh();
    return () => current.dispose();
  });

  onMount(() => {
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    void listen<{ conversation?: string }>('ops-update', (event) => {
      if (event.payload?.conversation && event.payload.conversation !== conversation) return;
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => void refresh(), 100);
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch((caught: unknown) => {
      if (!disposed) error = errorText(caught);
    });
    return () => {
      disposed = true;
      unlisten?.();
      clearTimeout(refreshTimer);
      gate.dispose();
      ++evidenceRequest;
    };
  });

  async function refresh(): Promise<void> {
    const id = conversation;
    if (!id) return;
    const current = gate;
    const request = current.begin();
    refreshing = true;
    try {
      const incoming = await opsTasks(id);
      if (!current.accepts(request) || conversation !== id) return;
      tasks = reconcileTasks(tasks, incoming, id);
      error = '';
      stale = false;
    } catch (caught) {
      if (current.accepts(request) && conversation === id) {
        error = errorText(caught);
        stale = tasks.length > 0;
      }
    } finally {
      if (current.accepts(request) && conversation === id) refreshing = false;
    }
  }

  function hostName(id: string): string { return hosts.find(id)?.name ?? id; }
  function currentContext(current: OpsRefreshGate, id: string): boolean { return current === gate && current.live && conversation === id; }

  function update(task: OpsTask): void {
    if (task.conversation !== conversation) return;
    const index = tasks.findIndex((row) => row.id === task.id);
    if (index === -1) tasks = [task, ...tasks];
    else if ((tasks[index]?.revision ?? -1) <= task.revision) tasks[index] = task;
  }

  async function append(task: OpsTask): Promise<void> {
    const text = inputs[task.id]?.trim();
    const hostIds = selectedTargets(task, choices[task.id]);
    if (!text || hostIds.length === 0 || busy[task.id]) return;
    const id = conversation;
    const current = gate;
    busy[task.id] = true;
    error = '';
    try {
      let verified = false;
      if (selectedUncertain(task, hostIds)) {
        verified = await ask({
          title: t('ops.confirmTitle'),
          message: t('ops.confirmMessage', { hosts: hostIds.filter((host) => isUncertain(subtaskFor(task, host))).map(hostName).join(', ') }),
          confirmLabel: t('ops.confirmContinue'),
          danger: true,
        });
        if (!verified || !currentContext(current, id)) return;
      }
      const updated = await opsTaskInput(task.id, text, hostIds, verified);
      if (!currentContext(current, id)) return;
      update(updated);
      inputs[task.id] = '';
      choices[task.id] = {};
      messages[task.id] = t('ops.queued');
      void refresh();
    } catch (caught) {
      if (currentContext(current, id)) error = errorText(caught);
    } finally {
      if (currentContext(current, id)) busy[task.id] = false;
    }
  }

  async function retry(task: OpsTask): Promise<void> {
    if (!canRetry(task) || busy[task.id]) return;
    const id = conversation;
    const current = gate;
    busy[task.id] = true;
    error = '';
    try {
      const updated = await opsTaskRetry(task.id);
      if (!currentContext(current, id)) return;
      update(updated);
      messages[task.id] = t('ops.retried');
      void refresh();
    } catch (caught) {
      if (currentContext(current, id)) error = errorText(caught);
    } finally {
      if (currentContext(current, id)) busy[task.id] = false;
    }
  }

  async function cancel(task: OpsTask): Promise<void> {
    if (!canCancel(task) || busy[task.id]) return;
    const id = conversation;
    const current = gate;
    busy[task.id] = true;
    error = '';
    try {
      await opsTaskCancel(task.id);
      if (!currentContext(current, id)) return;
      messages[task.id] = t('ops.cancelled');
      await refresh();
    } catch (caught) {
      if (currentContext(current, id)) error = errorText(caught);
    } finally {
      if (currentContext(current, id)) busy[task.id] = false;
    }
  }

  async function showEvidence(task: OpsTask, reference: PrivateRef, title: string): Promise<void> {
    const request = ++evidenceRequest;
    const id = conversation;
    evidence = { title, body: null, loading: true, error: '' };
    try {
      const body = await opsTaskEvidence(task.id, reference);
      if (request === evidenceRequest && conversation === id) evidence = { title, body, loading: false, error: '' };
    } catch (caught) {
      if (request === evidenceRequest && conversation === id) evidence = { title, body: null, loading: false, error: errorText(caught) };
    }
  }

  function closeEvidence(): void { ++evidenceRequest; evidence = null; }
</script>

<section class="tasks" aria-label={t('ops.title')}>
  <header><span><Icon name="files" size={13} />{t('ops.title')}</span><button class="link" type="button" onclick={refresh} disabled={refreshing}>{t('ops.refresh')}</button></header>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if stale}<p class="muted" role="status">{t('ops.stale')}</p>{/if}
  {#each tasks as task (task.id)}
    <details class="task">
      <summary><span>{t('ops.task', { id: task.id.slice(0, 8) })}</span><span class="state {task.state}">{t(`ops.state.${task.state}` as MessageKey)}</span></summary>
      <div class="task-body">
        <div class="metadata">
          <span>{t(`ops.source.${task.source.kind}` as MessageKey)}</span>
          {#if task.source.kind === 'schedule' || !task.scope.can_modify}
            <span title={t('ops.scopeHint')}>{task.scope.can_modify ? t('ops.scope.modify') : t('ops.scope.read')}</span>
          {/if}
          <span>{t('ops.updated', { time: new Date(task.updated_at).toLocaleString() })}</span>
        </div>
        <div class="evidence-links">
          {#if task.inputs[0]}<button class="link" type="button" onclick={() => task.inputs[0] && showEvidence(task, task.inputs[0], t('ops.request'))}>{t('ops.request')}</button>{/if}
          {#if task.acceptance}<button class="link" type="button" onclick={() => task.acceptance && showEvidence(task, task.acceptance, t('ops.acceptance'))}>{t('ops.acceptance')}</button>{/if}
          {#if task.report}<button class="link" type="button" onclick={() => task.report && showEvidence(task, task.report, t('ops.report'))}>{t('ops.report')}</button>{/if}
        </div>
        <fieldset>
          <legend>{t('ops.targets')}</legend>
          {#each task.scope.targets as target (target.host_id)}
            {@const subtask = subtaskFor(task, target.host_id)}
            {@const session = subtask?.session}
            <div class="target">
              <label class="check"><input type="checkbox" checked={selectedTargets(task, choices[task.id]).includes(target.host_id)} disabled={busy[task.id] || !canAppend(task)} onchange={(event) => choices[task.id] = { ...choices[task.id], [target.host_id]: targetChoice(task, target.host_id, event.currentTarget.checked) }} /><span>{hostName(target.host_id)}</span><span class="state {subtask?.state ?? 'queued'}">{t(`ops.state.${subtask?.state ?? 'queued'}` as MessageKey)}</span></label>
              {#if session && onselectsubagent}
                <button type="button" class="link" onclick={() => onselectsubagent?.(session, hostName(target.host_id))}>{t('ai.subagents.view')}</button>
              {/if}
              {#if subtask?.state === 'completed'}<p class="muted">{t('ops.completedTarget')}</p>{/if}
              {#if subtask?.reason}<p class="muted">{t(`ops.reason.${subtask.reason}` as MessageKey)}</p>{/if}
              {#if subtask?.error_code}<code class="error-code">{subtask.error_code}</code>{/if}
              {#if isUncertain(subtask)}<p class="attention">{t('ops.uncertain')}</p>{/if}
              {#if subtask?.state === 'waiting_approval'}<p class="muted">{t('ops.approvalHint')}</p>{/if}
              {#if subtask && subtask.evidence.length > 0}
                <div class="evidence-links">{#each subtask.evidence as reference, n (`${reference.session}:${reference.reference}`)}<button class="link" type="button" onclick={() => showEvidence(task, reference, `${hostName(target.host_id)} · ${t('ops.evidence', { n: n + 1 })}`)}>{t('ops.evidence', { n: n + 1 })}</button>{/each}</div>
              {/if}
            </div>
          {:else}
            <p class="muted">{t('ops.noTargets')}</p>
          {/each}
        </fieldset>
        {#if canAppend(task)}
          <form onsubmit={(event) => { event.preventDefault(); void append(task); }}>
            <label>{t('ops.input')}<textarea rows="2" value={inputs[task.id] ?? ''} oninput={(event) => inputs[task.id] = event.currentTarget.value} placeholder={t('ops.inputHint')} disabled={busy[task.id]}></textarea></label>
            <p class="muted">{t('ops.inputTargets')}</p>
            <button type="submit" class="secondary" disabled={busy[task.id] || !inputs[task.id]?.trim() || selectedTargets(task, choices[task.id]).length === 0}>{t('ops.send')}</button>
          </form>
        {/if}
        <div class="actions">
          {#if canRetry(task)}<button type="button" class="secondary" disabled={busy[task.id]} onclick={() => retry(task)}>{t('ops.retry')}</button>{/if}
          {#if canCancel(task)}<button type="button" class="secondary" disabled={busy[task.id]} onclick={() => cancel(task)}>{t('ops.cancel')}</button>{/if}
        </div>
        {#if messages[task.id]}<p class="feedback" role="status">{messages[task.id]}</p>{/if}
      </div>
    </details>
  {:else}
    <p class="muted">{refreshing ? t('ops.loading') : t('ops.empty')}</p>
  {/each}
</section>

{#if evidence}
  <Dialog title={evidence.title} onclose={closeEvidence} width={680}>
    {#if evidence.loading}<p class="muted">{t('ops.evidenceLoading')}</p>{:else if evidence.error}<p class="error" role="alert">{evidence.error}</p>{:else}
      {@const text = evidenceText(evidence.body)}
      {#if text !== null}<Markdown {text} />{/if}
      {#if typeof evidence.body !== 'string'}
        {#if text !== null}<details class="raw"><summary>{t('ops.rawEvidence')}</summary><pre class="selectable">{JSON.stringify(evidence.body, null, 2)}</pre></details>{:else}<pre class="selectable">{JSON.stringify(evidence.body, null, 2)}</pre>{/if}
      {/if}
    {/if}
    {#snippet actions()}<button type="button" class="secondary" onclick={closeEvidence}>{t('common.close')}</button>{/snippet}
  </Dialog>
{/if}

<style>
  .tasks { flex: none; max-height: 40vh; padding: var(--space-2) var(--space-3); overflow: auto; border-bottom: 1px solid var(--separator); font-size: var(--text-footnote); }
  header, header > span, .actions, .evidence-links, .metadata { display: flex; align-items: center; gap: var(--space-2); }
  header { justify-content: space-between; margin-bottom: var(--space-2); }
  header > span { font-weight: 500; }
  .task { margin-top: var(--space-2); border-radius: var(--radius-sm); background: var(--fill); }
  .task > summary { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); padding: var(--space-2); cursor: pointer; }
  .task-body { display: flex; flex-direction: column; gap: var(--space-2); padding: 0 var(--space-2) var(--space-3); }
  .metadata, .evidence-links, .actions { flex-wrap: wrap; }
  .metadata { color: var(--fg-muted); font-size: var(--text-caption); }
  .state { flex: none; color: var(--fg-muted); font-size: var(--text-caption); }
  .state.completed { color: var(--success); }
  .state.running, .state.queued { color: var(--accent); }
  .state.waiting_approval, .state.blocked, .attention { color: var(--warning); }
  .state.failed { color: var(--danger); }
  fieldset { margin: 0; padding: var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-sm); min-width: 0; }
  legend { padding: 0 var(--space-1); color: var(--fg-muted); }
  .target + .target { margin-top: var(--space-2); padding-top: var(--space-2); border-top: 1px solid var(--separator); }
  .check { display: flex; align-items: center; gap: var(--space-2); color: var(--fg); }
  .check .state { margin-left: auto; }
  .target p, .target .evidence-links { margin-top: var(--space-1); padding-left: 22px; }
  .error-code { display: block; margin-top: var(--space-1); color: var(--fg-muted); overflow-wrap: anywhere; }
  .muted, .attention, .feedback { margin: 0; font-size: var(--text-footnote); }
  .muted { color: var(--fg-muted); }
  .feedback { color: var(--accent); }
  .error { margin: var(--space-2) 0; color: var(--danger); overflow-wrap: anywhere; }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; cursor: pointer; text-align: left; }
  .link:disabled { opacity: 0.5; cursor: default; }
  form { display: flex; flex-direction: column; align-items: flex-start; gap: var(--space-2); }
  form label { display: flex; flex-direction: column; gap: var(--space-1); width: 100%; color: var(--fg-muted); }
  textarea { width: 100%; box-sizing: border-box; resize: vertical; font: inherit; }
  .raw summary { cursor: pointer; color: var(--fg-muted); }
  pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-family: var(--font-mono); font-size: var(--text-footnote); }
</style>
