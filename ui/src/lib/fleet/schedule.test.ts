import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clock, parseClock, repeatText } from './schedule.ts';

test('scheduled tasks show clocks and descriptions', () => {
  assert.equal(clock(9, 5), '09:05');
  assert.deepEqual(parseClock('9:05'), { hour: 9, minute: 5 });
  assert.equal(parseClock('24:00'), null);
  assert.deepEqual(repeatText({ kind: 'weekly', weekday: 0, hour: 9, minute: 0 }), { key: 'weekly', values: { day: 0, time: '09:00' } });
});
