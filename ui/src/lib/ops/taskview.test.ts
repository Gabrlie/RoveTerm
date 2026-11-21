import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canAppend, canCancel, canRetry, evidenceText, isUncertain, OpsRefreshGate, reconcileTasks, selectedTargets, selectedUncertain, targetChoice } from './taskview.ts';
import type { OpsSubtask, OpsTask, OpsTaskState } from './types';

function subtask(host: string, state: OpsTaskState, uncertain = false): OpsSubtask {
  return { id: `sub-${host}`, host_id: host, fingerprint: host, state, session: null, checkpoint: null, inputs: [], input_version: 0, evidence: [], reason: null, error_code: null, uncertain_effect: uncertain, attempts: 1 };
}

function task(subtasks: OpsSubtask[] = [], state: OpsTaskState = 'running'): OpsTask {
  return { id: 'task-one', revision: 1, request_id: 'request-one', conversation: 'conversation-one', source: { kind: 'user' }, state, scope: { targets: subtasks.map((sub) => ({ host_id: sub.host_id, fingerprint: sub.fingerprint })), can_modify: false }, inputs: [], acceptance: null, report: null, subtasks, created_at: 0, updated_at: 0 };
}

test('additional input defaults to unfinished hosts including running and waiting workers', () => {
  const value = task([subtask('running', 'running'), subtask('waiting', 'waiting_approval'), subtask('blocked', 'blocked'), subtask('done', 'completed')]);
  assert.deepEqual(selectedTargets(value), ['running', 'waiting', 'blocked']);
  assert.deepEqual(selectedTargets(value, { waiting: 'excluded', done: targetChoice(value, 'done', true) }), ['running', 'blocked', 'done']);
});

test('a host completing after it was selected needs a fresh explicit selection', () => {
  const running = task([subtask('one', 'running')]);
  const choice = targetChoice(running, 'one', true);
  assert.equal(choice, 'pending');
  const completed = task([subtask('one', 'completed')], 'completed');
  assert.deepEqual(selectedTargets(completed, { one: choice }), []);
  assert.deepEqual(selectedTargets(completed, { one: targetChoice(completed, 'one', true) }), ['one']);
  assert.deepEqual(selectedTargets(completed, {}), []);
});

test('uncertain effects require confirmation only when their host is selected', () => {
  const value = task([subtask('safe', 'interrupted'), subtask('uncertain', 'blocked', true)]);
  assert.equal(selectedUncertain(value, ['safe']), false);
  assert.equal(selectedUncertain(value, ['uncertain']), true);
  assert.equal(selectedUncertain(value, selectedTargets(value)), true);
});

test('an unresolved checkpoint effect requires confirmation even before the flag was persisted', () => {
  const interrupted = { ...subtask('interrupted', 'blocked'), reason: 'unresolved_effect' as const };
  const value = task([interrupted], 'blocked');
  assert.equal(interrupted.uncertain_effect, false);
  assert.equal(isUncertain(interrupted), true);
  assert.equal(selectedUncertain(value, ['interrupted']), true);
  assert.equal(selectedUncertain(value, []), false);
  assert.equal(canRetry(value), false);
  assert.equal(isUncertain(undefined), false);
  assert.equal(isUncertain(subtask('clean', 'interrupted')), false);
});

test('retry keeps completed work and unverified effects out of the automatic retry action', () => {
  assert.equal(canRetry(task([subtask('done', 'completed')], 'completed')), false);
  assert.equal(canRetry(task([subtask('uncertain', 'blocked', true)], 'blocked')), false);
  assert.equal(canRetry(task([subtask('uncertain', 'blocked', true), subtask('failed', 'failed')], 'blocked')), true);
  assert.equal(canRetry(task([subtask('failed', 'failed')], 'cancelled')), false);
  assert.equal(canCancel(task([], 'queued')), true);
  assert.equal(canCancel(task([], 'completed')), false);
  assert.equal(canCancel(task([], 'cancelled')), false);
});

test('restored interrupted and blocked tasks remain actionable, cancelled tasks do not append', () => {
  for (const state of ['interrupted', 'blocked', 'failed', 'completed'] as const) {
    assert.equal(canAppend(task([subtask('host', state)], state)), true);
  }
  assert.equal(canAppend(task([subtask('host', 'cancelled')], 'cancelled')), false);
  assert.equal(canAppend(task([], 'queued')), false);
});

test('older lists cannot replace newer task revisions or expose another conversation', () => {
  const latest = { ...task(), revision: 5, state: 'completed' as const };
  const older = { ...latest, revision: 2, state: 'running' as const };
  const foreign = { ...latest, id: 'other-task', conversation: 'another-conversation' };
  assert.deepEqual(reconcileTasks([latest], [older, foreign], 'conversation-one'), [latest]);
  const newer = { ...latest, revision: 6 };
  assert.deepEqual(reconcileTasks([latest], [newer], 'conversation-one'), [newer]);
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
});

test('evidence can render text while keeping structured data available separately', () => {
  assert.equal(evidenceText('# Report\nAll healthy'), '# Report\nAll healthy');
  assert.equal(evidenceText({ reply: 'Verified', stop: 'end_turn' }), 'Verified');
  assert.equal(evidenceText({ report: 'Report body' }), 'Report body');
  assert.equal(evidenceText({ text: 'Task request' }), 'Task request');
  assert.equal(evidenceText({ exit_code: 0, commands: [] }), null);
  assert.equal(evidenceText(null), null);
});
