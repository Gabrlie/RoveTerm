import type { OpsSubtask, OpsTask } from './types';

/** A completed host is selected only by a click made after it completed. */
export type TargetChoice = 'pending' | 'completed' | 'excluded';
export type TargetChoices = Record<string, TargetChoice>;

export function subtaskFor(task: OpsTask, hostId: string): OpsSubtask | undefined {
  return task.subtasks.find((subtask) => subtask.host_id === hostId);
}

export function selectedTargets(task: OpsTask, choices: TargetChoices = {}): string[] {
  return task.scope.targets.filter((target) => {
    const completed = subtaskFor(task, target.host_id)?.state === 'completed';
    return completed ? choices[target.host_id] === 'completed' : choices[target.host_id] !== 'excluded';
  }).map((target) => target.host_id);
}

export function targetChoice(task: OpsTask, hostId: string, checked: boolean): TargetChoice {
  return checked ? (subtaskFor(task, hostId)?.state === 'completed' ? 'completed' : 'pending') : 'excluded';
}

export function isUncertain(subtask?: Pick<OpsSubtask, 'uncertain_effect' | 'reason'>): boolean {
  return subtask?.uncertain_effect === true || subtask?.reason === 'unresolved_effect';
}

export function selectedUncertain(task: OpsTask, hostIds: string[]): boolean {
  return task.subtasks.some((subtask) => hostIds.includes(subtask.host_id) && isUncertain(subtask));
}

export function canRetry(task: OpsTask): boolean {
  return task.state !== 'cancelled' && task.subtasks.some((subtask) => ['failed', 'blocked', 'interrupted'].includes(subtask.state) && !isUncertain(subtask));
}

export function canCancel(task: OpsTask): boolean {
  return ['queued', 'running', 'waiting_approval', 'blocked', 'interrupted'].includes(task.state);
}

export function canAppend(task: OpsTask): boolean {
  return task.state !== 'cancelled' && task.scope.targets.length > 0;
}

/** An older snapshot cannot roll an already observed task back to an earlier revision. */
export function reconcileTasks(current: OpsTask[], incoming: OpsTask[], conversation: string): OpsTask[] {
  const existing = new Map(current.filter((task) => task.conversation === conversation).map((task) => [task.id, task]));
  return incoming.filter((task) => task.conversation === conversation).map((task) => {
    const previous = existing.get(task.id);
    return previous && previous.revision > task.revision ? previous : task;
  });
}

export function evidenceText(value: unknown): string | null {
  if (typeof value === 'string') return value;
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
  const fields = value as Record<string, unknown>;
  for (const key of ['text', 'report', 'reply']) {
    if (typeof fields[key] === 'string') return fields[key];
  }
  return null;
}

export class OpsRefreshGate {
  #revision = 0;
  #live = true;
  begin(): number { return ++this.#revision; }
  accepts(revision: number): boolean { return this.#live && revision === this.#revision; }
  get live(): boolean { return this.#live; }
  dispose(): void { this.#live = false; ++this.#revision; }
}
