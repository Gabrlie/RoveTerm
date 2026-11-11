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
