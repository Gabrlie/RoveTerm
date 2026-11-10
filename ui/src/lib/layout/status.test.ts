import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clock, durationKind, durationParts, untilNextMinute } from './status.ts';

test('durations split into days, hours and minutes', () => {
  assert.deepEqual(durationParts(30_000), { days: 0, hours: 0, minutes: 0 });
  assert.deepEqual(durationParts(3 * 3_600_000 + 12 * 60_000 + 5_000), { days: 0, hours: 3, minutes: 12 });
  assert.deepEqual(durationParts(26 * 3_600_000), { days: 1, hours: 2, minutes: 0 });
  assert.equal(durationKind(durationParts(30_000)), 'now');
  assert.equal(durationKind(durationParts(5 * 60_000)), 'minutes');
  assert.equal(durationKind(durationParts(2 * 3_600_000)), 'hours');
  assert.equal(durationKind(durationParts(50 * 3_600_000)), 'days');
});

test('the clock and its refresh', () => {
  const date = new Date(2026, 10, 10, 9, 5, 30, 250);
  assert.equal(clock(date), '09:05');
  assert.equal(untilNextMinute(date), 29_750);
});
