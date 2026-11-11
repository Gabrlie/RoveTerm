import { test } from 'node:test';
import assert from 'node:assert/strict';
import { consoleText } from './consolelog.ts';

test('console arguments read as one line of text', () => {
  assert.equal(consoleText(['load failed', { code: 'x' }]), 'load failed {"code":"x"}');
  const error = new Error('boom');
  assert.match(consoleText([error]), /^Error: boom\n/);
  assert.equal(consoleText(['a'.repeat(5000)]).length, 4096);
  const loop: Record<string, unknown> = {};
  loop.self = loop;
  assert.equal(consoleText([loop]), '[object Object]');
});
