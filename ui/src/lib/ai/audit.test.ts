import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { AiSessionSummary } from '../ipc';
import { auditLine, filterSessions, formatAuditDetail, formatSessionTime, groupSessionsByDate } from './audit.ts';

const at = (kind: string, payload: Record<string, unknown>) => auditLine({ ts: 0, kind, payload });

test('commands show their exit code and output', () => {
  const line = at('ai/command', { operation: { op: 'command', command: 'uptime' }, outcome: { result: 'command', exit_code: 0, stdout: 'up 3 days', stderr: '' } });
  assert.deepEqual(line, { kind: 'command', text: 'uptime', note: 'exit 0', detail: 'up 3 days', tone: 'normal', exitCode: 0 });
  assert.equal(at('ai/command', { outcome: { result: 'command', exit_code: null } })?.exitCode, null);
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
    kind: 'tool_call', text: 'task_status', note: 'completed', detail: JSON.stringify(input), tone: 'normal', toolSubject: '',
  });
  assert.equal(at('ai/tool_call', { status: 'completed', input: JSON.stringify(input) })?.text, '');
  assert.equal(at('ai/tool_call', { title: 'task_report', input: '{"host":"Staging"}' })?.toolSubject, 'Staging');
  assert.equal(at('ai/tool_call', { title: 'start_tasks', input: { title: 'Check disk space' } })?.toolSubject, 'Check disk space');
});

test('review rows keep the reviewer and rounded confidence for localization', () => {
  const line = at('ai/reviewed', { reviewer: 'model', confidence: 0.946, allowed: true });
  assert.equal(line?.reviewer, 'model');
  assert.equal(line?.confidence, 95);
  assert.equal(line?.note, 'model 95%');
  assert.equal(at('ai/reviewed', { reviewer: 'jev' })?.confidence, undefined);
});

test('message rows preview one clean line and keep the full Markdown', () => {
  const text = '\n ## Disk usage\n\n- 2 GB used';
  assert.equal(at('ai/message', { text })?.text, 'Disk usage');
  assert.equal(at('ai/message', { text })?.detail, text);
  assert.equal(at('ai/message', { text: '- First item\n- Second item' })?.text, 'First item');
  const long = '字'.repeat(160);
  assert.equal(at('ai/message', { text: long })?.text, `${long.slice(0, 139)}…`);
});

test('command audit details show output, errors or the command', () => {
  const operation = { command: 'uptime' };
  assert.deepEqual(formatAuditDetail('ai/command', { operation, outcome: { stdout: 'up 3 days\n', stderr: 'notice' } }),
    { kind: 'text', text: 'up 3 days\n\nnotice' });
  assert.deepEqual(formatAuditDetail('command', { operation, outcome: { error: 'Host unavailable' } }),
    { kind: 'text', text: 'Host unavailable' });
  assert.deepEqual(formatAuditDetail('command', { operation }), { kind: 'text', text: 'uptime' });
});

test('file, terminal and approval audit details show readable content', () => {
  const diff = '- old\n+ new';
  for (const kind of ['file_write', 'notes_write']) {
    assert.deepEqual(formatAuditDetail(kind, { operation: { diff }, outcome: { content: 'new' } }), { kind: 'diff', text: diff });
    assert.deepEqual(formatAuditDetail(kind, { outcome: { content: 'new' } }), { kind: 'text', text: 'new' });
  }
  for (const kind of ['file_read', 'terminal_read', 'notes_read', 'list_dir']) {
    assert.deepEqual(formatAuditDetail(kind, { outcome: { content: '{"a":1}' } }), { kind: 'text', text: '{"a":1}' });
    assert.deepEqual(formatAuditDetail(kind, { outcome: { error: 'Unavailable' } }), { kind: 'text', text: 'Unavailable' });
  }
  assert.deepEqual(formatAuditDetail('approval', { asked: { diff } }), { kind: 'diff', text: diff });
  assert.deepEqual(formatAuditDetail('ai/approval_asked', { asked: { command: 'uptime', detail: 'Check uptime' } }),
    { kind: 'text', text: 'uptime' });
  assert.deepEqual(formatAuditDetail('approval_asked', { asked: { detail: 'Check uptime' } }), { kind: 'text', text: 'Check uptime' });
});

test('tool details prefer output and format JSON input', () => {
  const input = { host: 'Staging' };
  const json = { kind: 'json', text: JSON.stringify(input, null, 2) };
  assert.deepEqual(formatAuditDetail('ai/tool_call', { input }), json);
  assert.deepEqual(formatAuditDetail('tool_call', { input: JSON.stringify(input) }), json);
  assert.deepEqual(formatAuditDetail('tool_call', { input: {}, outcome: { content: JSON.stringify(input) } }), json);
  assert.deepEqual(formatAuditDetail('tool_call', { input, outcome: { content: 'No reports' } }), { kind: 'text', text: 'No reports' });
  assert.deepEqual(formatAuditDetail('tool_call', { outcome: { content: ' true ' } }), { kind: 'json', text: 'true' });
});

test('plain strings, JSON strings and other audit payloads have readable formats', () => {
  assert.deepEqual(formatAuditDetail('tool_call', '  [1,2]  '), { kind: 'json', text: '[\n  1,\n  2\n]' });
  assert.deepEqual(formatAuditDetail('file_read', '{not JSON}'), { kind: 'text', text: '{not JSON}' });
  assert.deepEqual(formatAuditDetail('command', 'line 1\nline 2'), { kind: 'text', text: 'line 1\nline 2' });
  assert.deepEqual(formatAuditDetail('unknown', { summary: 'Summary' }), { kind: 'text', text: 'Summary' });
  assert.deepEqual(formatAuditDetail('unknown', { value: 3 }), { kind: 'json', text: '{\n  "value": 3\n}' });
  assert.deepEqual(formatAuditDetail('unknown', null), { kind: 'text', text: '' });
  assert.deepEqual(formatAuditDetail('unknown', 42), { kind: 'text', text: '42' });
});

function session(id: string, started: number, fields: Partial<AiSessionSummary> = {}): AiSessionSummary {
  return { id, started, role: 'assistant', title: null, host: null, agent: null, model: null, children: [], agent_ref: null, context: null, ...fields };
}

test('history groups sessions at local midnight and preserves input order within a bucket', () => {
  const now = new Date(2026, 10, 18, 12).getTime();
  const midnight = new Date(2026, 10, 18).getTime();
  const yesterday = new Date(2026, 10, 17).getTime();
  const week = new Date(2026, 10, 12).getTime();
  const recent = session('now', now);
  const earlier = session('midnight', midnight);
  const lastNight = session('last-night', midnight - 1);
  const thisWeek = session('six-days-ago', week);
  const older = session('older', week - 1);
  assert.deepEqual(groupSessionsByDate([earlier, older, lastNight, thisWeek, recent, session('yesterday', yesterday)], now), [
    { bucket: 'today', sessions: [earlier, recent] },
    { bucket: 'yesterday', sessions: [lastNight, session('yesterday', yesterday)] },
    { bucket: 'week', sessions: [thisWeek] },
    { bucket: 'older', sessions: [older] },
  ]);
  assert.deepEqual(groupSessionsByDate([thisWeek], now), [{ bucket: 'week', sessions: [thisWeek] }]);
  assert.deepEqual(groupSessionsByDate([], now), []);
});

test('history search matches titles, host labels, agents, models and child hosts', () => {
  const child = session('child', 0, { role: 'subagent', host: 'Backup (ops@vault)' });
  const fleet = session('fleet', 0, { role: 'fleet', title: 'Check disk space', children: [child] });
  const assistant = session('assistant', 0, { host: 'Production (root@edge)', model: 'Mini', agent: 'Codex' });
  const list = [fleet, assistant];
  assert.equal(filterSessions(list, '  '), list);
  for (const query of ['DISK SPACE', 'backup', 'ops@vault']) assert.deepEqual(filterSessions(list, query), [fleet]);
  for (const query of ['production', 'root@edge', 'CODEx', ' mini ']) assert.deepEqual(filterSessions(list, query), [assistant]);
  assert.deepEqual(filterSessions(list, 'missing'), []);
});

test('history time uses local 24-hour time and the relevant date fields', () => {
  const now = new Date(2026, 0, 2, 15, 4).getTime();
  assert.equal(formatSessionTime(new Date(2026, 0, 2, 0, 5).getTime(), now), '00:05');
  assert.equal(formatSessionTime(new Date(2026, 0, 1, 17, 6).getTime(), now), '01/01 17:06');
  assert.equal(formatSessionTime(new Date(2025, 11, 31, 23, 59).getTime(), now), '2025/12/31');
});
