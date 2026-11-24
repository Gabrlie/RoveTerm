import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeVisualViewportIme, sanitizeInsets } from './insets.ts';

const zero = { top: 0, right: 0, bottom: 0, left: 0, imeBottom: 0 };

test('defaults absent insets to zero', () => {
  assert.deepEqual(sanitizeInsets(null), zero);
  assert.deepEqual(sanitizeInsets(undefined), zero);
  assert.deepEqual(sanitizeInsets({ left: 12 }), { ...zero, left: 12 });
});

test('clamps invalid insets and rounds fractional CSS pixels', () => {
  assert.deepEqual(sanitizeInsets({ top: NaN, right: Infinity, bottom: -4, left: 13.4, imeBottom: 240.6 }), {
    ...zero,
    left: 13,
    imeBottom: 241,
  });
});

test('measures the gap below the visual viewport after accounting for its offset', () => {
  assert.equal(computeVisualViewportIme(800, 520.4, 20), 260);
  assert.equal(computeVisualViewportIme(800, 520, 0), 280);
});

test('returns zero for an already resized layout or an invalid viewport measurement', () => {
  assert.equal(computeVisualViewportIme(520, 520, 0), 0);
  assert.equal(computeVisualViewportIme(520, 521, 0), 0);
  assert.equal(computeVisualViewportIme(520, NaN, 0), 0);
  assert.equal(computeVisualViewportIme(Infinity, 520, 0), 0);
});
