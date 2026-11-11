import { test } from 'node:test';
import assert from 'node:assert/strict';
import { contextPercent, contextTokens, speed, tokens, wait, windowName } from './quota.ts';

const window = (minutes: number | null, label = '') => ({ label, minutes, used_percent: 10, resets_at: null, resets_text: null });

test('windows are named by their length or the agent', () => {
  assert.equal(windowName(window(300)).key, 'fiveHours');
  assert.equal(windowName(window(299)).key, 'fiveHours');
  assert.equal(windowName(window(10080)).key, 'week');
  assert.deepEqual(windowName(window(2880)), { key: 'days', count: 2, label: '' });
  assert.deepEqual(windowName(window(null, '5-hour limit')), { key: null, count: 0, label: '5-hour limit' });
});

test('context usage', () => {
  assert.equal(contextPercent({ used: 50_000, size: 200_000 }), 25);
  assert.equal(contextPercent({ used: 1, size: 0 }), null);
  assert.equal(contextPercent(null), null);
  assert.equal(tokens(123_456), '123k');
  assert.equal(tokens(1_000_000), '1M');
  assert.equal(tokens(512), '512');
});

test('request stats read as speed, wait and context', () => {
  assert.equal(speed(42.4), '42 tok/s');
  assert.equal(speed(7.25), '7.3 tok/s');
  assert.equal(wait(850), '850 ms');
  assert.equal(wait(1234), '1.2 s');
  assert.equal(contextTokens({ input_tokens: 1000, output_tokens: 20 }), 1020);
  assert.equal(contextTokens({ input_tokens: null, output_tokens: 20 }), null);
  assert.equal(contextTokens(null), null);
});
