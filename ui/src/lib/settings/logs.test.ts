import test from 'node:test';
import assert from 'node:assert/strict';
import { filterLogEntries, formatLogSize, parseLogLines } from './logs.ts';

const sample = [
  '2026-11-22T01:00:00.123456Z  INFO roveterm_lib: starting version="0.1.0"',
  '2026-11-22T01:00:01.456Z  WARN rove_sync::webdav: webdav request failed error="dns error"',
  '  caused by: failed to lookup address',
  '2026-11-22T01:00:02.789Z ERROR ui: unexpected state',
  '2026-11-22T01:00:03.000Z DEBUG rove_sync::webdav: webdav response status=207',
  '',
].join('\n');

test('detects tracing levels and preserves severity on continuation lines', () => {
  const entries = parseLogLines(sample);
  assert.equal(entries.length, 5);
  assert.deepEqual(
    entries.map((entry) => entry.level),
    ['INFO', 'WARN', 'WARN', 'ERROR', 'DEBUG'],
  );
});

test('filters log entries by level and keyword', () => {
  const entries = parseLogLines(sample);
  assert.equal(filterLogEntries(entries, 'ALL', '').length, 5);
  assert.equal(filterLogEntries(entries, 'WARN', '').length, 2);
  assert.equal(filterLogEntries(entries, 'ERROR', '').length, 1);
  assert.equal(filterLogEntries(entries, 'DEBUG', '').length, 1);
  assert.equal(filterLogEntries(entries, 'ALL', 'WEBDAV').length, 2);
  assert.equal(filterLogEntries(entries, 'WARN', 'lookup').length, 1);
});

test('parses timestamps, tracing targets, and messages while retaining raw lines', () => {
  const entries = parseLogLines(sample);
  assert.deepEqual(entries[0], {
    index: 1,
    raw: '2026-11-22T01:00:00.123456Z  INFO roveterm_lib: starting version="0.1.0"',
    level: 'INFO',
    timestamp: '2026-11-22T01:00:00.123456Z',
    shortTime: '01:00:00.123',
    target: 'roveterm_lib',
    message: 'starting version="0.1.0"',
    continuation: false,
  });
  assert.equal(entries[1]?.target, 'rove_sync::webdav');
  assert.equal(entries[1]?.message, 'webdav request failed error="dns error"');
  assert.equal(entries[3]?.target, 'ui');
  assert.deepEqual(entries[2], {
    index: 3,
    raw: '  caused by: failed to lookup address',
    level: 'WARN',
    timestamp: null,
    shortTime: null,
    target: null,
    message: '  caused by: failed to lookup address',
    continuation: true,
  });
});

test('handles offset timestamps, span context, and lines without a timestamp or target', () => {
  const entries = parseLogLines([
    '2026-11-22T01:00:01.4+08:00 WARN ts_runtime::multiderp::uniderp: run{region_id=1}: DERP disconnected',
    '2026-11-22 01:00:02Z DEBUG opening connection',
    'TRACE https://example.invalid/',
  ].join('\r\n'));
  assert.equal(entries[0]?.timestamp, '2026-11-22T01:00:01.4+08:00');
  assert.equal(entries[0]?.shortTime, '01:00:01.400');
  assert.equal(entries[0]?.target, 'ts_runtime::multiderp::uniderp');
  assert.equal(entries[0]?.message, 'run{region_id=1}: DERP disconnected');
  assert.equal(entries[1]?.shortTime, '01:00:02.000');
  assert.equal(entries[1]?.target, null);
  assert.equal(entries[1]?.message, 'opening connection');
  assert.equal(entries[2]?.timestamp, null);
  assert.equal(entries[2]?.shortTime, null);
  assert.equal(entries[2]?.target, null);
  assert.equal(entries[2]?.message, 'https://example.invalid/');
  assert.equal(filterLogEntries(entries, 'DEBUG', '').length, 2);
});

test('keeps plain and blank lines intact and ends severity inheritance at blank lines', () => {
  const entries = parseLogLines('plain line\nERROR ui: failure\n  first cause\n\nplain again\n');
  assert.deepEqual(entries.map(({ level, continuation, message }) => [level, continuation, message]), [
    ['OTHER', false, 'plain line'],
    ['ERROR', false, 'failure'],
    ['ERROR', true, '  first cause'],
    ['OTHER', false, ''],
    ['OTHER', false, 'plain again'],
  ]);
  assert.deepEqual(parseLogLines(''), []);
});

test('strips stray ANSI CSI escape sequences from older log files', () => {
  const polluted = '2026-11-23T08:57:23.368Z  INFO actor.lifecycle{\u001b[3mactor.name\u001b[0m\u001b[2m=\u001b[0m"ts_runtime::derp_latency::DerpLatencyMeasurer" \u001b[3mactor.id\u001b[0m\u001b[2m=\u001b[0m#23}: ts_runtime::derp_latency: selected home DERP region_id=10 latency_ms=395';
  const [entry] = parseLogLines(polluted);
  assert.ok(entry);
  assert.equal(entry.level, 'INFO');
  assert.equal(entry.shortTime, '08:57:23.368');
  assert.equal(entry.raw.includes('\u001b'), false);
  assert.equal(
    entry.message,
    'actor.lifecycle{actor.name="ts_runtime::derp_latency::DerpLatencyMeasurer" actor.id=#23}: ts_runtime::derp_latency: selected home DERP region_id=10 latency_ms=395',
  );
});

test('formats byte sizes cleanly', () => {
  assert.equal(formatLogSize(512), '512 B');
  assert.equal(formatLogSize(2048), '2.0 KB');
  assert.equal(formatLogSize(1_572_864), '1.50 MB');
});
