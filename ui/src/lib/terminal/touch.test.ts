import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  accumulateScrollLines, alternateScrollSequence, exceededTouchSlop,
  FONT_SIZE_MAX, FONT_SIZE_MIN, LONG_PRESS_MS, pinchFontSize, touchDistance, TOUCH_MOVE_SLOP,
} from './touch.ts';

test('touch distance uses both axes and is symmetric', () => {
  const a = { x: 5, y: -2 };
  const b = { x: 2, y: 2 };
  assert.equal(touchDistance(a, b), 5);
  assert.equal(touchDistance(b, a), 5);
  assert.equal(touchDistance(a, a), 0);
});

test('pinch ratios enlarge, shrink, round and clamp the font', () => {
  assert.equal(pinchFontSize(14, 100, 150), 21);
  assert.equal(pinchFontSize(18, 100, 75), 14);
  assert.equal(pinchFontSize(18, 100, 60), 11);
  assert.equal(pinchFontSize(18, 100, 70), 13);
  assert.equal(pinchFontSize(15, 100, 10), FONT_SIZE_MIN);
  assert.equal(pinchFontSize(15, 100, 300), FONT_SIZE_MAX);
  assert.equal(pinchFontSize(15, 0, 10), 15);
  assert.equal(pinchFontSize(33, -1, 10), FONT_SIZE_MAX);
});

test('dragging upward advances the viewport and dragging downward reveals older lines', () => {
  assert.deepEqual(accumulateScrollLines(0, -25, 10), { lines: 2, remainderPx: 5 });
  assert.deepEqual(accumulateScrollLines(0, 25, 10), { lines: -2, remainderPx: -5 });
  assert.deepEqual(accumulateScrollLines(0, 10, 10), { lines: -1, remainderPx: 0 });
});

test('scroll accumulation retains subline motion and reverses smoothly', () => {
  const first = accumulateScrollLines(0, 7, 12.5);
  assert.deepEqual(first, { lines: 0, remainderPx: -7 });
  const second = accumulateScrollLines(first.remainderPx, 8, 12.5);
  assert.deepEqual(second, { lines: -1, remainderPx: -2.5 });
  assert.deepEqual(accumulateScrollLines(second.remainderPx, -16, 12.5), { lines: 1, remainderPx: 1 });
  assert.deepEqual(accumulateScrollLines(0, 0, 12.5), { lines: 0, remainderPx: 0 });
  assert.deepEqual(accumulateScrollLines(0, 20, 0), { lines: 0, remainderPx: 0 });
  assert.deepEqual(accumulateScrollLines(0, 20, Number.POSITIVE_INFINITY), { lines: 0, remainderPx: 0 });
});

test('alternate scrolling sends up for history and down for forward with a per-tick cap', () => {
  assert.equal(alternateScrollSequence(-2), '\x1b[A\x1b[A');
  assert.equal(alternateScrollSequence(3), '\x1b[B\x1b[B\x1b[B');
  assert.equal(alternateScrollSequence(-50), '\x1b[A'.repeat(10));
  assert.equal(alternateScrollSequence(10.8), '\x1b[B'.repeat(10));
  assert.equal(alternateScrollSequence(0), '');
});

test('slop allows a stationary hold and rejects motion in every direction past the threshold', () => {
  assert.equal(LONG_PRESS_MS, 450);
  assert.equal(TOUCH_MOVE_SLOP, 10);
  assert.equal(exceededTouchSlop(0, 0, 0, 0), false);
  assert.equal(exceededTouchSlop(0, 0, 6, 8), false);
  assert.equal(exceededTouchSlop(0, 0, -8, -8), true);
  assert.equal(exceededTouchSlop(0, 0, 11, 0), true);
  assert.equal(exceededTouchSlop(0, 0, 0, -11), true);
  assert.equal(exceededTouchSlop(2, 2, 2, 6, 3), true);
});
