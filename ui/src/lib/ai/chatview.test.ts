import { test } from 'node:test';
import assert from 'node:assert/strict';
import { approvalFor, outcomeState, rows, type ChatItem } from './chatview.ts';

const readOnly = { risk: 'read_only' as const, programs: [], opaque: false, reasons: [] };
const ran = (command: string, exit = 0): ChatItem => ({
  kind: 'operation',
  operation: { op: 'command', command },
  assessment: readOnly,
  approved: false,
  outcome: { result: 'command', output: { stdout: '', stderr: '', exit_code: exit, truncated: false } },
});

test('consecutive quiet operations fold into a group', () => {
  const items: ChatItem[] = [{ kind: 'user', text: 'check' }, ran('uptime'), ran('df -h'), ran('free'), { kind: 'agent', text: 'ok', messageId: null }, ran('ls')];
  const laid = rows(items);
  assert.deepEqual(laid.map((row) => row.kind), ['item', 'group', 'item', 'item']);
  assert.equal(laid[1]?.kind === 'group' && laid[1].items.length, 3);
});

test('failures and approved operations stay on their own', () => {
  const failed = ran('false', 1);
  const laid = rows([ran('uptime'), failed, ran('df')]);
  assert.deepEqual(laid.map((row) => row.kind), ['item', 'item', 'item']);
});

test('a result finds the approval it belongs to', () => {
  const operation = { op: 'write_file' as const, path: '/a' };
  const approval: ChatItem = { kind: 'approval', requestId: 1, operation, assessment: readOnly, rememberable: true, agentTool: false, answer: 'allow_once', outcome: null };
  const waiting: ChatItem = { ...approval, requestId: 2, answer: undefined };
  assert.equal(approvalFor([approval, waiting], operation), approval);
  assert.equal(approvalFor([waiting], operation), undefined);
  assert.equal(approvalFor([approval], { op: 'write_file', path: '/b' }), undefined);
});

test('outcome states', () => {
  assert.equal(outcomeState(null), 'running');
  assert.equal(outcomeState({ result: 'denied' }), 'denied');
  assert.equal(outcomeState({ result: 'command', output: { stdout: '', stderr: '', exit_code: 2, truncated: false } }), 'failed');
  assert.equal(outcomeState({ result: 'written', bytes: 1, sha256: '', previous_sha256: null, lines_before: 0, lines_after: 1 }), 'done');
});
