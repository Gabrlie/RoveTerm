import type { SubtaskEvent } from '../ipc';
import type { ChatItem } from './chatview';

export type SubagentStart = Extract<SubtaskEvent, { task_id: string }>;
export type SubagentSelection = { parentId: string; id: string };

interface SubagentState {
  parentId: string | null;
  hostId: string | null;
  hostLabel: string | null;
  taskId: string | null;
  subtaskId: string | null;
  taskPrompt: string;
  historical: boolean;
  busy: boolean;
  ended: boolean;
  items: ChatItem[];
}

export function startSubagent<T extends SubagentState>(
  subagents: Map<string, T>,
  event: SubagentStart,
  create: () => T,
  leadItems?: ChatItem[],
): T {
  const chat = subagents.get(event.id) ?? create();
  chat.parentId = event.parent;
  chat.hostId = event.host_id;
  chat.hostLabel = event.host;
  chat.taskId = event.task_id;
  chat.subtaskId = event.subtask_id;
  chat.taskPrompt = event.task;
  chat.historical = false;
  chat.ended = false;
  chat.busy = true;
  chat.items.push({ kind: 'delegation', task: event.task });
  subagents.set(event.id, chat);

  if (leadItems) {
    const card = leadItems.find((item) => item.kind === 'subtask' && item.id === event.id);
    if (card?.kind === 'subtask') Object.assign(card, { task: event.task, host: event.host });
    else leadItems.push({ kind: 'subtask', id: event.id, task: event.task, host: event.host });
  }
  return chat;
}

export function purgeSubagents<T extends Pick<SubagentState, 'parentId'>>(
  subagents: Map<string, T>,
  byId: Record<string, T>,
  parents: ReadonlySet<string>,
  selected: SubagentSelection | null,
): SubagentSelection | null {
  for (const [id, chat] of subagents) {
    if (!chat.parentId || !parents.has(chat.parentId)) continue;
    subagents.delete(id);
    if (byId[id] === chat) delete byId[id];
  }
  return selected && parents.has(selected.parentId) ? null : selected;
}

export function waitingCount(chats: Iterable<{ waiting: number }>): number {
  let count = 0;
  for (const chat of chats) count += chat.waiting;
  return count;
}

export function firstWaitingChat<T extends { waiting: number }>(chats: Iterable<T>, except?: T): T | null {
  for (const chat of chats) if (chat !== except && chat.waiting > 0) return chat;
  return null;
}

export function subagentStatus(chat: { historical: boolean; ended: boolean; waiting: number }): 'history' | 'finished' | 'waiting' | 'running' {
  if (chat.historical) return 'history';
  if (chat.ended) return 'finished';
  return chat.waiting > 0 ? 'waiting' : 'running';
}
