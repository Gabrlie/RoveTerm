import { SvelteMap } from 'svelte/reactivity';
import { t } from '../i18n';
import { isCommandError, onOpsUpdate, opsSubtaskCancel, opsTaskCancel, opsTaskRetry, opsTasks } from '../ipc';
import { activeWorkerCount, isTaskActive, OpsRefreshGate, reconcileTasks, upsertTask } from './taskview';
import type { OpsTaskView } from './types';

function errorKey(error: unknown) {
  if (isCommandError(error)) {
    switch (error.code) {
      case 'ops.store': return 'error.ops.store';
      case 'ops.checkpoint': return 'error.ops.checkpoint';
      case 'input.invalid':
      case 'ops.invalid': return 'error.ops.invalid';
      case 'ops.private': return 'error.ops.private';
    }
  }
  return 'error.ops.failed';
}

class OpsStore {
  private tasks = new SvelteMap<string, OpsTaskView[]>();
  private errors = new SvelteMap<string, ReturnType<typeof errorKey>>();
  private loading = new Map<string, { gate: OpsRefreshGate; pending: Promise<void> }>();
  private deleted = new Set<string>();
  private listener: Promise<void> | undefined;

  listen(): Promise<void> {
    return this.listener ??= onOpsUpdate((task) => this.upsert(task)).then(() => undefined).catch((error: unknown) => {
      this.listener = undefined;
      console.warn('Task updates unavailable', error);
    });
  }

  load(conversation: string): Promise<void> {
    if (this.deleted.has(conversation)) return Promise.resolve();
    const existing = this.loading.get(conversation);
    if (existing) return existing.pending;
    const gate = new OpsRefreshGate();
    const pending = this.refresh(conversation, gate);
    this.loading.set(conversation, { gate, pending });
    return pending;
  }

  private async refresh(conversation: string, gate: OpsRefreshGate): Promise<void> {
    const request = gate.begin();
    try {
      await this.listen();
      if (!gate.accepts(request)) return;
      const incoming = await opsTasks(conversation);
      if (!gate.accepts(request)) return;
      this.tasks.set(conversation, reconcileTasks(this.tasksOf(conversation), incoming, conversation));
      this.errors.delete(conversation);
    } catch (error) {
      if (gate.accepts(request)) this.errors.set(conversation, errorKey(error));
    } finally {
      if (gate.accepts(request)) this.loading.delete(conversation);
    }
  }

  tasksOf(conversation: string | null): OpsTaskView[] {
    return (conversation ? this.tasks.get(conversation) : undefined) ?? [];
  }

  errorOf(conversation: string | null): string {
    const key = conversation ? this.errors.get(conversation) : undefined;
    return key ? t(key) : '';
  }

  hasActive(conversation: string | null): boolean {
    return this.tasksOf(conversation).some(isTaskActive);
  }

  activeWorkers(conversation: string | null): number {
    return activeWorkerCount(this.tasksOf(conversation));
  }

  subtaskBySession(conversation: string | null, sessionId: string) {
    for (const task of this.tasksOf(conversation)) {
      const sub = task.subtasks.find((candidate) => candidate.session === sessionId);
      if (sub) return sub;
    }
  }

  private upsert(task: OpsTaskView): OpsTaskView {
    const conversation = task.conversation;
    if (!this.deleted.has(conversation)) {
      const current = this.tasksOf(conversation);
      const next = upsertTask(current, task, conversation);
      if (next !== current) this.tasks.set(conversation, next);
    }
    return task;
  }

  async cancelTask(id: string): Promise<OpsTaskView> {
    return this.upsert(await opsTaskCancel(id));
  }

  async cancelSubtask(taskId: string, subtaskId: string): Promise<OpsTaskView> {
    return this.upsert(await opsSubtaskCancel(taskId, subtaskId));
  }

  async retryTask(id: string, hostId?: string | null, confirmUncertain?: boolean): Promise<OpsTaskView> {
    return this.upsert(await opsTaskRetry(id, hostId, confirmUncertain));
  }

  clearConversation(conversation: string): void {
    this.deleted.add(conversation);
    this.loading.get(conversation)?.gate.dispose();
    this.loading.delete(conversation);
    this.tasks.delete(conversation);
    this.errors.delete(conversation);
  }
}

export const ops = new OpsStore();
