import type { OpsSubtaskView, OpsTaskState, OpsTaskView } from './types';

type TaskActivity = Pick<OpsTaskView, 'state' | 'subtasks'>;

export function isActiveState(state: OpsTaskState): boolean {
  return state === 'queued' || state === 'running' || state === 'waiting_approval';
}

export function isTaskActive(task: TaskActivity): boolean {
  return isActiveState(task.state) || task.subtasks.some(isSubtaskActive);
}

export function isSubtaskActive(sub: Pick<OpsSubtaskView, 'state'>): boolean {
  return isActiveState(sub.state);
}

export function canRetrySubtask(sub: Pick<OpsSubtaskView, 'state'>): boolean {
  return sub.state === 'failed' || sub.state === 'blocked' || sub.state === 'interrupted';
}

export function canRetryTask(task: TaskActivity): boolean {
  return task.state !== 'cancelled' && task.subtasks.some(canRetrySubtask);
}

export function canCancelTask(task: TaskActivity): boolean {
  return isTaskActive(task);
}

export function uncertainHosts(task: OpsTaskView, hostId?: string | null): string[] {
  return task.subtasks
    .filter((sub) => (hostId == null || sub.host_id === hostId) && canRetrySubtask(sub) && sub.uncertain_effect)
    .map((sub) => sub.host_name);
}

export function activeWorkerCount(tasks: readonly OpsTaskView[]): number {
  return tasks.reduce((count, task) => count + task.subtasks.filter(isSubtaskActive).length, 0);
}

function newestFirst(a: OpsTaskView, b: OpsTaskView): number {
  return b.updated_at - a.updated_at;
}

/** Keeps task events received while a snapshot was in flight. */
export function reconcileTasks(current: OpsTaskView[], incoming: OpsTaskView[], conversation: string): OpsTaskView[] {
  const tasks = new Map(current.filter((task) => task.conversation === conversation).map((task) => [task.id, task]));
  for (const task of incoming) {
    if (task.conversation !== conversation) continue;
    const previous = tasks.get(task.id);
    if (!previous || previous.revision <= task.revision) tasks.set(task.id, task);
  }
  return [...tasks.values()].sort(newestFirst);
}

export function upsertTask(current: OpsTaskView[], incoming: OpsTaskView, conversation: string): OpsTaskView[] {
  if (incoming.conversation !== conversation) return current;
  const previous = current.find((task) => task.id === incoming.id);
  if (previous && previous.revision > incoming.revision) return current;
  return [incoming, ...current.filter((task) => task.id !== incoming.id)].sort(newestFirst);
}

export function shortHostName(label: string | null | undefined): string {
  return label?.replace(/\s+\([^()\s@]+@[^()\s@]+\)$/, '') ?? '';
}

export class OpsRefreshGate {
  #revision = 0;
  #live = true;
  begin(): number { return ++this.#revision; }
  accepts(revision: number): boolean { return this.#live && revision === this.#revision; }
  get live(): boolean { return this.#live; }
  dispose(): void { this.#live = false; ++this.#revision; }
}
