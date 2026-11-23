import { test } from 'node:test';
import assert from 'node:assert/strict';
import { itemsFromRecords, approvalFor, waitingApproval, commandOf, connectionProblem, formatToolOutput, outcomeState, rows, subject, toolSubject, type ChatItem, type LeadToolName, type ToolItem } from './chatview.ts';

test('history keeps body references and uses the execution reference on a completed approval', () => {
  const command = { op: 'command', command: 'uptime' };
  const outcome = { result: 'command', exit_code: 0 };
  const items = itemsFromRecords([
    { ts: 0, kind: 'ai/command', payload: { operation: command, outcome, body_ref: 'standalone' } },
    { ts: 1, kind: 'ai/approval', payload: { request_id: 1, asked: command, body_ref: 'ask' } },
    { ts: 2, kind: 'ai/approval', payload: { request_id: 1, answer: 'allow_once' } },
    { ts: 3, kind: 'ai/command', payload: { operation: command, outcome, body_ref: 'execution' } },
    { ts: 4, kind: 'ai/tool_call', payload: { id: 'one', title: 'list_hosts', body_ref: 'tool' } },
    { ts: 5, kind: 'ai/approval', payload: { request_id: 2, asked: command, body_ref: 'withdrawn-ask' } },
    { ts: 6, kind: 'ai/command', payload: { operation: command, outcome, body_ref: '' } },
    { ts: 7, kind: 'ai/tool_call', payload: { id: 'two' } },
    { ts: 8, kind: 'ai/approval', payload: { request_id: 3, asked: command, body_ref: false } },
  ]);

  assert.deepEqual(items.map((item) => item.kind), ['operation', 'approval', 'tool', 'approval', 'operation', 'tool', 'approval']);
  assert.deepEqual(items.map((item) => 'bodyRef' in item ? item.bodyRef : null),
    ['standalone', 'execution', 'tool', 'withdrawn-ask', null, null, null]);
});

const readOnly = { risk: 'read_only' as const, programs: [], opaque: false, reasons: [] };
const ran = (command: string, exit = 0): ChatItem => ({
  kind: 'operation',
  operation: { op: 'command', command },
  assessment: readOnly,
  approved: false,
  outcome: { result: 'command', output: { stdout: '', stderr: '', exit_code: exit, truncated: false } },
});
const leadTool = (title: LeadToolName, raw_input: unknown = null, status = 'completed'): ToolItem => ({
  kind: 'tool',
  call: { id: title, title, kind: 'other', status, content: [], locations: [], raw_input },
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

test('completed Lead reads fold together and with successful read-only operations', () => {
  const first = leadTool('list_hosts');
  const tools = [first, leadTool('read_notes'), leadTool('task_status'), leadTool('task_report')];
  assert.deepEqual(rows(tools), [{ kind: 'group', items: tools, key: 0 }]);
  const mixed = [ran('uptime'), first];
  assert.deepEqual(rows(mixed), [{ kind: 'group', items: mixed, key: 0 }]);
});

test('unfinished, failed and mutating Lead tools interrupt read-only groups', () => {
  for (const tool of [
    leadTool('read_notes', null, 'failed'),
    leadTool('task_report', null, 'in_progress'),
    leadTool('start_tasks'),
    leadTool('cancel_task'),
    leadTool('retry_task'),
    leadTool('task_input'),
  ]) {
    const items = [leadTool('list_hosts'), tool, leadTool('task_status')];
    assert.deepEqual(rows(items), items.map((item, key) => ({ kind: 'item', item, key })), tool.call.title ?? undefined);
  }
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
  assert.equal(toolSubject({ ...call, raw_input: { command: 'pwd' } }), 'pwd');
  assert.equal(subject({ op: 'agent_tool', kind: 'edit', title: 'Edit', command: null, path: '/a/b' }), '/a/b');
  assert.equal(subject({ op: 'agent_tool', kind: 'other', title: 'Approve x', command: null }), '');
});

test('Lead tools show readable hosts and task titles', () => {
  const label = (name: LeadToolName, input: unknown) => toolSubject(leadTool(name, input).call);
  assert.equal(label('read_notes', { host: ' edge ' }), 'edge');
  assert.equal(label('task_report', { host: 'db', id: '0199abcd-1234-7000-8000-123456789abc' }), 'db');
  assert.equal(label('task_report', { host: ' ', id: '0199abcd-1234-7000-8000-123456789abc' }), '');
  assert.equal(label('start_tasks', { title: ' Check disks ', tasks: [{ host: 'edge' }] }), 'Check disks');
  assert.equal(label('start_tasks', { title: ' ', tasks: [{ host: 'edge' }, { host: ' edge ' }, null, { host: ' ' }, { host: 'db' }] }), 'edge, db');
  assert.equal(label('start_tasks', { tasks: ['a', 'b', 'c', 'd', 'e'].map((host) => ({ host })) }), 'a, b, c +2');
  assert.equal(label('task_input', { hosts: ['edge', ' edge ', 'db', '', 1] }), 'edge, db');
  assert.equal(label('task_input', { hosts: ['a', 'b', 'c', 'd'] }), 'a, b, c +1');
  for (const name of ['list_hosts', 'task_status', 'retry_task', 'cancel_task'] as const) {
    assert.equal(label(name, { id: '0199abcd-1234-7000-8000-123456789abc' }), '', name);
  }
});

test('recorded Lead tools keep their subject when the input object is available', () => {
  const [item] = itemsFromRecords([
    { ts: 1, kind: 'ai/tool_call', payload: { title: 'start_tasks', status: 'completed', input: { title: 'Check disks' } } },
  ]);
  assert.ok(item?.kind === 'tool');
  assert.equal(toolSubject(item.call), 'Check disks');
});

test('tool output indents JSON objects and arrays and preserves other text', () => {
  assert.equal(formatToolOutput('  {"host":"edge"}\n'), '{\n  "host": "edge"\n}');
  assert.equal(formatToolOutput('[1,2]'), '[\n  1,\n  2\n]');
  assert.equal(formatToolOutput(' uptime\n'), ' uptime\n');
  assert.equal(formatToolOutput(' {not JSON} '), ' {not JSON} ');
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

test('a sub-agent stays a row of its own between reads', () => {
  const read = (path: string): ChatItem => ({ kind: 'operation', operation: { op: 'read_file', path }, assessment: readOnly, approved: false, outcome: { result: 'read', bytes: 1, sha256: '', truncated: false } });
  const laid = rows([read('/a'), { kind: 'subtask', id: 'ai-2', host: 'web', task: 'check' }, read('/b')]);
  assert.deepEqual(laid.map((row) => (row.kind === 'item' ? row.item.kind : row.kind)), ['operation', 'subtask', 'operation']);
});

test('a sub-agent history shows each prompt as a delegated task', () => {
  const records = [
    { ts: 1, kind: 'ai/prompt', payload: { text: 'Check the disk' } },
    { ts: 2, kind: 'ai/message', payload: { text: 'The disk is healthy.' } },
    { ts: 3, kind: 'ai/prompt', payload: { text: 'Check free inodes' } },
  ];
  assert.deepEqual(itemsFromRecords(records, { asSubagent: true }), [
    { kind: 'delegation', task: 'Check the disk' },
    { kind: 'agent', text: 'The disk is healthy.', messageId: null },
    { kind: 'delegation', task: 'Check free inodes' },
  ]);
  assert.equal(itemsFromRecords(records)[0]?.kind, 'user');
});

test('repeated approval IDs in worker history are answered in their own turns', () => {
  const asked = (path: string) => ({ request_id: 1, asked: { op: 'write_file', path }, risk: 'modify' });
  const items = itemsFromRecords([
    { ts: 1, kind: 'ai/approval', payload: asked('/a') },
    { ts: 2, kind: 'ai/approval', payload: { request_id: 1, answer: 'deny' } },
    { ts: 3, kind: 'ai/approval', payload: asked('/b') },
    { ts: 4, kind: 'ai/approval', payload: { request_id: 1, answer: 'allow_once' } },
  ], { asSubagent: true });
  assert.deepEqual(items.map((item) => item.kind === 'approval' && item.answer), ['deny', 'allow_once']);
});
