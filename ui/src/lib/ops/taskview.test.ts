import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  activeWorkerCount, canCancelTask, canRetrySubtask, canRetryTask, isActiveState, isSubtaskActive,
  isTaskActive, OpsRefreshGate, reconcileTasks, shortHostName, uncertainHosts, upsertTask,
} from './taskview.ts';
import type { OpsSubtaskView, OpsTaskState, OpsTaskView } from './types';

function subtask(host: string, state: OpsTaskState, uncertain = false): OpsSubtaskView {
  return {
    id: `sub-${host}`, host_id: host, host_name: `Host ${host}`, state, session: null, task: 'Check disk',
    summary: null, reply: null, checks: [], reason: null, error: null, uncertain_effect: uncertain, attempts: 1,
  };
}

function task(overrides: Partial<OpsTaskView> = {}): OpsTaskView {
  const subtasks = overrides.subtasks ?? [];
  const counts = {
    total: subtasks.length, queued: 0, running: 0, waiting_approval: 0, completed: 0, blocked: 0,
    failed: 0, cancelled: 0, interrupted: 0,
  };
  for (const sub of subtasks) counts[sub.state] += 1;
  return {
    id: 'task-one', conversation: 'lead', revision: 1, title: 'Check disks', source: { kind: 'user' },
    state: 'running', read_only: true, created_at: 0, updated_at: 0, counts, subtasks, ...overrides,
  };
}

test('activity, cancellation and retry helpers classify every state', () => {
  const states = [
    ['queued', true, false],
    ['running', true, false],
    ['waiting_approval', true, false],
    ['blocked', false, true],
    ['completed', false, false],
    ['failed', false, true],
    ['cancelled', false, false],
    ['interrupted', false, true],
  ] as const;
  for (const [state, active, retryable] of states) {
    const sub = subtask(state, state);
    const view = task({ state, subtasks: [sub] });
    assert.equal(isActiveState(state), active, state);
    assert.equal(isSubtaskActive(sub), active, state);
    assert.equal(isTaskActive(view), active, state);
    assert.equal(canCancelTask(view), active, state);
    assert.equal(canRetrySubtask(sub), retryable, state);
    assert.equal(canRetryTask(view), retryable, state);
  }
});

test('mixed host states keep a task active and let unfinished hosts retry', () => {
  const failed = subtask('failed', 'failed');
  const awaiting = subtask('awaiting', 'waiting_approval');
  assert.equal(isTaskActive(task({ state: 'queued' })), true);
  assert.equal(isTaskActive(task({ state: 'blocked', subtasks: [failed, awaiting] })), true);
  assert.equal(canRetryTask(task({ state: 'running', subtasks: [failed, awaiting] })), true);
  assert.equal(canRetryTask(task({ state: 'cancelled', subtasks: [failed] })), false);
});

test('retry confirmation names only selected retryable hosts with uncertain effects', () => {
  const view = task({ subtasks: [
    subtask('failed', 'failed', true),
    subtask('blocked', 'blocked', true),
    subtask('safe', 'interrupted'),
    subtask('settled', 'completed', true),
    subtask('queued', 'queued', true),
  ] });
  assert.deepEqual(uncertainHosts(view), ['Host failed', 'Host blocked']);
  assert.deepEqual(uncertainHosts(view, null), ['Host failed', 'Host blocked']);
  assert.deepEqual(uncertainHosts(view, 'blocked'), ['Host blocked']);
  assert.deepEqual(uncertainHosts(view, 'safe'), []);
  assert.deepEqual(uncertainHosts(view, 'missing'), []);
});

test('worker count includes queued, running and awaiting hosts across tasks', () => {
  assert.equal(activeWorkerCount([]), 0);
  assert.equal(activeWorkerCount([
    task({ subtasks: [subtask('queued', 'queued'), subtask('blocked', 'blocked')] }),
    task({ subtasks: [subtask('running', 'running'), subtask('waiting', 'waiting_approval'), subtask('done', 'completed')] }),
    task({ state: 'queued' }),
  ]), 3);
});

test('refresh preserves newer and newly emitted tasks, filters by conversation and sorts by update time', () => {
  const latest = task({ revision: 5, updated_at: 10, state: 'completed' });
  const older = task({ revision: 2, updated_at: 1 });
  const foreign = task({ id: 'other-task', conversation: 'other' });
  const fromEvent = task({ id: 'new-task', updated_at: 30 });
  const fromSnapshot = task({ id: 'old-task', updated_at: 8 });
  assert.deepEqual(reconcileTasks([latest, fromEvent, foreign], [older, fromSnapshot, foreign], 'lead'), [fromEvent, latest, fromSnapshot]);
  const newer = task({ revision: 6, updated_at: 40 });
  assert.deepEqual(reconcileTasks([latest, fromEvent], [newer], 'lead'), [newer, fromEvent]);
  const enriched = { ...newer, title: 'Report available' };
  assert.deepEqual(reconcileTasks([newer], [enriched], 'lead'), [enriched]);
});

test('upsert ignores stale or foreign events and inserts updates once in date order', () => {
  const current = [task({ id: 'second', updated_at: 20 }), task({ revision: 5, updated_at: 10 })];
  assert.equal(upsertTask(current, task({ revision: 4, updated_at: 30 }), 'lead'), current);
  assert.equal(upsertTask(current, task({ conversation: 'other' }), 'lead'), current);
  const added = task({ id: 'added', updated_at: 25 });
  assert.deepEqual(upsertTask(current, added, 'lead'), [added, ...current]);
  const updated = task({ revision: 6, updated_at: 30 });
  assert.deepEqual(upsertTask(current, updated, 'lead'), [updated, current[0]]);
  assert.deepEqual(upsertTask(current, { ...updated, updated_at: 5 }, 'lead'), [current[0], { ...updated, updated_at: 5 }]);
});

test('compact host labels strip only a trailing user and host suffix', () => {
  assert.equal(shortHostName('Database (prod) (root@db.local:2222)'), 'Database (prod)');
  assert.equal(shortHostName('Database (root@[fd00::1])'), 'Database');
  assert.equal(shortHostName('Database (Primary)'), 'Database (Primary)');
  assert.equal(shortHostName('Database (root@db) copy'), 'Database (root@db) copy');
  assert.equal(shortHostName('root@db'), 'root@db');
  assert.equal(shortHostName(null), '');
  assert.equal(shortHostName(undefined), '');
});

test('request generations reject late refreshes and results after disposal', () => {
  const gate = new OpsRefreshGate();
  const before = gate.begin();
  const after = gate.begin();
  assert.equal(gate.accepts(before), false);
  assert.equal(gate.accepts(after), true);
  assert.equal(gate.live, true);
  gate.dispose();
  assert.equal(gate.live, false);
  assert.equal(gate.accepts(after), false);
  assert.equal(gate.accepts(gate.begin()), false);
});
