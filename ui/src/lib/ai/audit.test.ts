import { test } from 'node:test';
import assert from 'node:assert/strict';
import { auditLine } from './audit.ts';

const at = (kind: string, payload: Record<string, unknown>) => auditLine({ ts: 0, kind, payload });

test('commands show their exit code and output', () => {
  const line = at('ai/command', { operation: { op: 'command', command: 'uptime' }, outcome: { result: 'command', exit_code: 0, stdout: 'up 3 days', stderr: '' } });
  assert.deepEqual(line, { kind: 'command', text: 'uptime', note: 'exit 0', detail: 'up 3 days', tone: 'normal' });
});

test('approvals show the risk asked and the answer', () => {
  assert.equal(at('ai/approval', { asked: { op: 'command', command: 'rm -rf /' }, risk: 'dangerous' })?.tone, 'danger');
  assert.equal(at('ai/approval', { answer: null })?.note, 'withdrawn');
  assert.equal(at('ai/approval', { answer: 'deny' })?.tone, 'danger');
});

test('refusals and unknown kinds', () => {
  assert.equal(at('ai/file_write', { operation: { op: 'write_file', path: '/etc/hosts' }, outcome: { result: 'denied' } })?.tone, 'danger');
  assert.equal(at('ai/thought', { text: 'hmm' }), null);
  assert.equal(at('ai/something_new', {}), null);
});

test('an edit shows its change', () => {
  const line = auditLine({ ts: 1, kind: 'ai/file_write', payload: { operation: { op: 'edit_file', path: '/etc/a', diff: '- a\n+ b\n' }, outcome: { result: 'written' } } });
  assert.equal(line?.text, '/etc/a');
  assert.equal(line?.detail, '- a\n+ b\n');
});

test('summary wakeups keep their input in the expandable detail', () => {
  assert.deepEqual(at('ai/ops_wakeup', { text: 'Two hosts finished.' }), {
    kind: 'ops_wakeup', text: '', note: '', detail: 'Two hosts finished.', tone: 'muted',
  });
});

test('Lead tool audit rows keep their title and status with input in the detail', () => {
  const input = { id: '0199abcd-1234-7000-8000-123456789abc' };
  assert.deepEqual(at('ai/tool_call', { title: 'task_status', status: 'completed', input }), {
    kind: 'tool_call', text: 'task_status', note: 'completed', detail: JSON.stringify(input), tone: 'normal',
  });
  assert.equal(at('ai/tool_call', { status: 'completed', input: JSON.stringify(input) })?.text, '');
});
