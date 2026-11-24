import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cycleFlatTarget, flatTabAt, flattenTabs } from './flatTabs.ts';

const first = { groupId: 'left', paneId: 'one' };
const second = { groupId: 'left', paneId: 'two' };
const third = { groupId: 'right', paneId: 'three' };
const tabs = [first, second, third];

test('an empty workspace has only Home', () => {
  assert.deepEqual(flattenTabs([], {}), []);
  assert.deepEqual(flattenTabs(['empty'], { empty: { tabs: [] } }), []);
  assert.equal(cycleFlatTarget([], null, 1), null);
  assert.equal(cycleFlatTarget([], null, -1), null);
  assert.equal(flatTabAt([], 0), null);
});

test('flattening follows group reading order and preserves each group’s tab order', () => {
  const groupIds = Object.freeze(['left', 'right']);
  const groups = Object.freeze({
    right: { tabs: Object.freeze(['three']) },
    left: { tabs: Object.freeze(['one', 'two']) },
  });
  assert.deepEqual(flattenTabs(groupIds, groups), tabs);
});

test('flattening skips missing groups', () => {
  assert.deepEqual(flattenTabs(['missing', 'right'], { right: { tabs: ['three'] } }), [third]);
});

test('cycling wraps forward and backward through Home', () => {
  assert.deepEqual(cycleFlatTarget(tabs, null, 1), first);
  assert.deepEqual(cycleFlatTarget(tabs, second.paneId, 1), third);
  assert.equal(cycleFlatTarget(tabs, third.paneId, 1), null);
  assert.deepEqual(cycleFlatTarget(tabs, null, -1), third);
  assert.deepEqual(cycleFlatTarget(tabs, third.paneId, -1), second);
  assert.equal(cycleFlatTarget(tabs, first.paneId, -1), null);
});

test('cycling handles large offsets and falls back to Home for a stale active pane', () => {
  assert.deepEqual(cycleFlatTarget(tabs, first.paneId, 4), first);
  assert.deepEqual(cycleFlatTarget(tabs, null, -5), third);
  assert.deepEqual(cycleFlatTarget(tabs, 'missing', 1), first);
});

test('indexed lookup is zero-based and spans groups', () => {
  assert.deepEqual(flatTabAt(tabs, 0), first);
  assert.deepEqual(flatTabAt(tabs, 1), second);
  assert.deepEqual(flatTabAt(tabs, 2), third);
  assert.equal(flatTabAt(tabs, -1), null);
  assert.equal(flatTabAt(tabs, 3), null);
});
