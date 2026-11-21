import test from 'node:test';
import assert from 'node:assert/strict';
import { filterLogEntries, formatLogSize, parseLogLines } from './logs.ts';

const sample = [
  '2026-11-22T01:00:00.123Z  INFO roveterm_lib: starting version="0.1.0"',
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

test('formats byte sizes cleanly', () => {
  assert.equal(formatLogSize(512), '512 B');
  assert.equal(formatLogSize(2048), '2.0 KB');
  assert.equal(formatLogSize(1_572_864), '1.50 MB');
});
