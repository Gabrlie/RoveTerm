import type { SubtaskEndState, SubtaskEvent } from '../ipc';
import type { OpsTaskState } from '../ops/types';
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
  finalState: SubtaskEndState | null;
  summary: string | null;
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
  chat.finalState = null;
  chat.summary = null;
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

export type SubagentStatus = 'running' | 'waiting' | 'completed' | 'failed' | 'blocked' | 'cancelled' | 'history';

export function isSubagentActive(chat: Pick<SubagentState, 'historical' | 'ended'>): boolean {
  return !chat.historical && !chat.ended;
}

type StatusChat = Pick<SubagentState, 'historical' | 'ended' | 'finalState'> & { waiting: number };

function terminalStatus(state?: OpsTaskState | null): SubagentStatus | undefined {
  switch (state) {
    case 'interrupted': return 'blocked';
    case 'blocked':
    case 'failed':
    case 'cancelled':
    case 'completed': return state;
  }
}

export function subagentStatus(chat: StatusChat, opsState?: OpsTaskState): SubagentStatus {
  if (chat.historical) return terminalStatus(opsState) ?? 'history';
  if (!chat.ended) return chat.waiting > 0 ? 'waiting' : 'running';
  return terminalStatus(opsState) ?? terminalStatus(chat.finalState) ?? 'completed';
}

export function latestActivity(items: readonly ChatItem[]): string | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item?.kind === 'delegation') break;
    let detail: string | null = null;
    if (item?.kind === 'tool') detail = item.call.title;
    if (item?.kind === 'approval' || item?.kind === 'operation') {
      const operation = item.operation;
      if (operation.op === 'agent_tool') detail = operation.command || operation.path || operation.title;
      else if ('command' in operation) detail = operation.command;
      else if ('path' in operation) detail = operation.path;
    }
    const line = detail?.trim().split('\n', 1)[0]?.trim();
    if (line) {
      const chars = [...line];
      return chars.length > 60 ? `${chars.slice(0, 59).join('')}…` : line;
    }
  }
  return null;
}
