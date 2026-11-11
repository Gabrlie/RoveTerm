import { test } from 'node:test';
import assert from 'node:assert/strict';
import { itemsFromRecords, approvalFor, waitingApproval, commandOf, connectionProblem, outcomeState, rows, subject, toolSubject, type ChatItem } from './chatview.ts';

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

test('connection problems are told apart from replies', () => {
  assert.ok(connectionProblem('Falling back from WebSockets to HTTPS transport. stream disconnected before completion: 由于目标计算机积极拒绝，无法连接。 (os error 10061)'));
  assert.ok(connectionProblem('Reconnecting... 2/5'));
  assert.ok(connectionProblem('The model provider reported an error: error sending request for url'));
  assert.ok(!connectionProblem('The disk is 80% full; the connection pool looks fine.'));
});

test('agent tools show their command or file', () => {
  assert.equal(commandOf({ command: ['/bin/bash', '-lc', 'df -h'] }), 'df -h');
  assert.equal(commandOf({ command: ['git', 'commit', '-m', "it's done"] }), "git commit -m 'it'\\''s done'");
  assert.equal(commandOf({ cmd: 'ls' }), 'ls');
  assert.equal(commandOf({ path: '/a' }), null);
  assert.equal(commandOf('ls'), null);
  const call = { id: 't', title: 'Read', kind: 'read', status: null, content: [], locations: ['/etc/hosts'], raw_input: null };
  assert.equal(toolSubject(call), '/etc/hosts');
  assert.equal(subject({ op: 'agent_tool', kind: 'edit', title: 'Edit', command: null, path: '/a/b' }), '/a/b');
  assert.equal(subject({ op: 'agent_tool', kind: 'other', title: 'Approve x', command: null }), '');
});

test('an allowed agent tool shows once, as its tool row', () => {
  const operation = { op: 'agent_tool' as const, kind: 'execute', title: 'Run make', command: 'make' };
  const approval = (answer: 'allow_once' | 'deny'): ChatItem => ({ kind: 'approval', requestId: 1, operation, assessment: readOnly, rememberable: false, agentTool: true, answer, outcome: null });
  const tool: ChatItem = { kind: 'tool', call: { id: 'x', title: 'Run make', kind: 'execute', status: 'completed', content: [], locations: [], raw_input: null } };
  assert.deepEqual(rows([tool, approval('allow_once')]).map((row) => row.kind === 'item' && row.item.kind), ['tool']);
  assert.deepEqual(rows([tool, approval('deny')]).map((row) => row.kind === 'item' && row.item.kind), ['tool', 'approval']);
});

test('an earlier conversation is rebuilt from its records', () => {
  const operation = { op: 'write_file', path: '/etc/motd' };
  const items = itemsFromRecords([
    { ts: 1, kind: 'ai/session_started', payload: { agent: 'X' } },
    { ts: 2, kind: 'ai/prompt', payload: { text: 'check' } },
    { ts: 3, kind: 'ai/message', payload: { text: 'Looking.' } },
    { ts: 4, kind: 'ai/command', payload: { operation: { op: 'command', command: 'uptime' }, risk: 'read_only', approved: false, outcome: { result: 'command', exit_code: 0, stdout: 'up', stderr: '', truncated: false } } },
    { ts: 5, kind: 'ai/approval', payload: { request_id: 7, asked: operation, risk: 'modify', reasons: [], agent_tool: false } },
    { ts: 6, kind: 'ai/approval', payload: { request_id: 7, answer: 'allow_once' } },
    { ts: 7, kind: 'ai/file_write', payload: { operation, risk: 'modify', approved: true, outcome: { result: 'written', bytes: 3 } } },
    { ts: 8, kind: 'ai/tool_call', payload: { id: 't', title: 'Read x', kind: 'read', status: 'completed', locations: ['/x'], input: '{"command":"cat /x"}' } },
    { ts: 9, kind: 'ai/approval', payload: { request_id: 8, asked: { op: 'command', command: 'reboot' }, risk: 'dangerous', reasons: [] } },
    { ts: 10, kind: 'ai/turn_ended', payload: { stop_reason: 'error: rate limited' } },
  ]);
  assert.deepEqual(items.map((item) => item.kind), ['user', 'agent', 'operation', 'approval', 'tool', 'approval', 'notice']);
  const command = items[2];
  assert.ok(command?.kind === 'operation' && command.outcome.result === 'command' && command.outcome.output.stdout === 'up');
  const write = items[3];
  assert.ok(write?.kind === 'approval' && write.answer === 'allow_once' && write.outcome?.result === 'written');
  const tool = items[4];
  assert.ok(tool?.kind === 'tool' && toolSubject(tool.call) === 'cat /x');
  const unanswered = items[5];
  assert.ok(unanswered?.kind === 'approval' && unanswered.answer === null);
});

test('connection problems of an earlier conversation show as such', () => {
  const warning = 'Falling back from WebSockets to HTTPS transport. stream disconnected before completion (os error 10061)';
  const items = itemsFromRecords([
    { ts: 1, kind: 'ai/prompt', payload: { text: 'hi' } },
    { ts: 2, kind: 'ai/message', payload: { text: warning } },
    { ts: 3, kind: 'ai/turn_ended', payload: { stop_reason: 'error: stream disconnected before completion (os error 10061)' } },
  ]);
  assert.deepEqual(items.map((item) => item.kind), ['user', 'connection']);
  assert.ok(items[1]?.kind === 'connection' && items[1].state === 'failed');
});

test('answers in a continued session reach its own approvals', () => {
  const old = itemsFromRecords([
    { ts: 1, kind: 'ai/approval', payload: { request_id: 1, asked: { op: 'command', command: 'a' }, risk: 'modify', reasons: [] } },
    { ts: 2, kind: 'ai/approval', payload: { request_id: 1, answer: 'deny' } },
  ]);
  assert.ok(old[0]?.kind === 'approval' && old[0].requestId === -1);
  const fresh: ChatItem = { kind: 'approval', requestId: 1, operation: { op: 'command', command: 'b' }, assessment: readOnly, rememberable: false, agentTool: false, answer: undefined, outcome: null };
  const items = [...old, fresh];
  assert.equal(waitingApproval(items, 1), fresh);
  assert.equal(waitingApproval([{ ...fresh, answer: 'allow_once' }], 1), undefined);
});

test('a compaction is rebuilt from its record', () => {
  const items = itemsFromRecords([{ ts: 1, kind: 'ai/compacted', payload: { summary: 'earlier' } }]);
  assert.deepEqual(items, [{ kind: 'compacted', summary: 'earlier' }]);
});

test('an operation the reviewer let run keeps its reviewer', () => {
  const operation = { op: 'write_file', path: '/etc/motd' };
  const items = itemsFromRecords([
    { ts: 1, kind: 'ai/reviewed', payload: { operation, reviewer: 'jev', allowed: true, confidence: 0.9 } },
    { ts: 2, kind: 'ai/file_write', payload: { operation, risk: 'modify', approved: false, outcome: { result: 'written', bytes: 1, sha256: '', previous_sha256: null, lines_before: 0, lines_after: 1 } } },
    { ts: 3, kind: 'ai/file_write', payload: { operation, risk: 'modify', approved: false, outcome: { result: 'written', bytes: 1, sha256: '', previous_sha256: null, lines_before: 0, lines_after: 1 } } },
  ]);
  assert.equal(items[0]?.kind === 'operation' ? items[0].reviewer : 'x', 'jev');
  assert.equal(items[1]?.kind === 'operation' ? items[1].reviewer : 'x', undefined);
});

test('only successful reads fold together', () => {
  const read = (path: string): ChatItem => ({ kind: 'operation', operation: { op: 'read_file', path }, assessment: readOnly, approved: false, outcome: { result: 'read', bytes: 1, sha256: '', truncated: false } });
  const write: ChatItem = { kind: 'operation', operation: { op: 'write_file', path: '/c' }, assessment: { ...readOnly, risk: 'modify' }, approved: false, outcome: { result: 'written', bytes: 1, sha256: '', previous_sha256: null, lines_before: 0, lines_after: 1 }, reviewer: 'model' };
  const laid = rows([read('/a'), read('/b'), write]);
  assert.deepEqual(laid.map((row) => row.kind), ['group', 'item']);
});
