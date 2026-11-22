import { test } from 'node:test';
import assert from 'node:assert/strict';
import { addCache, cacheRate, contextMeter, contextPercent, contextTokens, speed, tokens, usageTone, wait, windowName } from './quota.ts';

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

test('a configured context size is visible at zero before the first request', () => {
  assert.deepEqual(contextMeter(null, 128_000, null), {
    context: { used: 0, size: 128_000 }, percent: 0, filled: null, tone: 'normal',
  });
  const stats = { input_tokens: 3_000, output_tokens: 500 };
  assert.equal(contextMeter(null, 10_000, stats).percent, 35);
  assert.equal(contextMeter({ used: 5_000, size: 10_000 }, 128_000, stats).percent, 50);
});

test('context displays warning and danger at 80 and 95 percent', () => {
  assert.equal(usageTone(79.99), 'normal');
  assert.equal(contextMeter({ used: 80, size: 100 }, null, null).tone, 'warning');
  assert.equal(usageTone(94.99), 'warning');
  assert.equal(contextMeter({ used: 95, size: 100 }, null, null).tone, 'danger');
  assert.equal(contextMeter({ used: 150, size: 100 }, null, null).percent, 100);
});

test('context falls back to tokens or unknown when the window is unavailable', () => {
  const stats = { input_tokens: 8_000, output_tokens: 12 };
  assert.deepEqual(contextMeter(null, null, stats), { context: null, percent: null, filled: 8012, tone: 'normal' });
  assert.equal(contextMeter({ used: 6_500, size: 0 }, null, stats).filled, 6_500);
  assert.deepEqual(contextMeter(null, null, null), { context: null, percent: null, filled: null, tone: 'normal' });
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

test('cache hits are a share of the input, summed over reporting requests', () => {
  let totals = { input: 0, cached: 0 };
  totals = addCache(totals, { input_tokens: 100, cached_tokens: 80 });
  totals = addCache(totals, { input_tokens: 50, cached_tokens: null });
  totals = addCache(totals, { input_tokens: 100, cached_tokens: 100 });
  assert.deepEqual(totals, { input: 200, cached: 180 });
  assert.equal(cacheRate(totals.cached, totals.input), 90);
  assert.equal(cacheRate(null, 100), null);
  assert.equal(cacheRate(0, 0), null);
});
