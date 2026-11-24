import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isCompactWidth, isKeyboardOpen } from './viewport.ts';

test('uses the compact layout below 768 pixels', () => {
  assert.equal(isCompactWidth(767), true);
  assert.equal(isCompactWidth(768), false);
});

test('detects the keyboard only above 40 finite pixels', () => {
  for (const value of [0, 40, NaN, Infinity, -5]) {
    assert.equal(isKeyboardOpen(value), false);
  }
  assert.equal(isKeyboardOpen(41), true);
});
