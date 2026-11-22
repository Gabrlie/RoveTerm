import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { ChatItem } from './chatview.ts';
import { firstWaitingChat, purgeSubagents, startSubagent, subagentStatus, waitingCount, type SubagentStart } from './subagent.ts';

function worker() {
  return {
    parentId: null as string | null,
    hostId: null as string | null,
    hostLabel: null as string | null,
    taskId: null as string | null,
    subtaskId: null as string | null,
    taskPrompt: '',
    historical: true,
    ended: true,
    busy: false,
    waiting: 0,
    items: [] as ChatItem[],
  };
}

const dispatch: SubagentStart = {
  id: 'worker', parent: 'lead', host: 'Database', host_id: 'db',
  task_id: 'task', subtask_id: 'subtask', task: 'Check disk usage',
};

test('dispatch keeps the worker separate and retries update the original lead card', () => {
  const lead = { items: [] as ChatItem[] };
  const topLevel = new Map([['fleet', lead]]);
  const subagents = new Map<string, ReturnType<typeof worker>>();
  const first = startSubagent(subagents, dispatch, worker, lead.items);
  first.items.push({ kind: 'agent', text: 'Disk is healthy', messageId: null });
  first.ended = true;
  first.busy = false;

  const second = startSubagent(subagents, { ...dispatch, task: 'Check free inodes', host: 'DB' }, worker, lead.items);
  assert.equal(second, first);
  assert.equal(second.ended, false);
  assert.equal(second.busy, true);
  assert.equal(second.historical, false);
  assert.equal(second.hostId, 'db');
  assert.equal(second.taskId, 'task');
  assert.equal(second.subtaskId, 'subtask');
  assert.equal(second.taskPrompt, 'Check free inodes');
  assert.deepEqual(second.items.map((item) => item.kind), ['delegation', 'agent', 'delegation']);
  assert.deepEqual(lead.items, [{ kind: 'subtask', id: 'worker', host: 'DB', task: 'Check free inodes' }]);
  assert.deepEqual([...topLevel.keys()], ['fleet']);
  startSubagent(subagents, { ...dispatch, id: 'other-worker' }, worker, lead.items);
  assert.equal(lead.items.length, 2);
});

test('lead replacement clears both live and resumed workers and their selection', () => {
  const live = { ...worker(), parentId: 'lead' };
  const resumed = { ...worker(), parentId: 'old-lead' };
  const other = { ...worker(), parentId: 'unrelated' };
  const subagents = new Map([['worker', live], ['archived', resumed], ['other', other]]);
  const byId = { worker: live, other, archived: other };

  const selected = purgeSubagents(subagents, byId, new Set(['lead', 'old-lead']), { parentId: 'lead', id: 'worker' });
  assert.equal(selected, null);
  assert.deepEqual([...subagents.keys()], ['other']);
  assert.deepEqual(byId, { other, archived: other });
});

test('waiting workers contribute to totals and the next approval targets a different worker', () => {
  const settled = { ...worker(), historical: false, waiting: 0 };
  const first = { ...worker(), historical: false, ended: false, waiting: 2 };
  const next = { ...worker(), historical: false, ended: false, waiting: 1 };
  const children = [settled, first, next];
  assert.equal(waitingCount(children) + waitingCount([{ waiting: 1 }]), 4);
  assert.equal(firstWaitingChat(children), first);
  assert.equal(firstWaitingChat(children, first), next);
  assert.equal(subagentStatus(first), 'waiting');
  assert.equal(subagentStatus(settled), 'finished');
  assert.equal(subagentStatus(worker()), 'history');
});
