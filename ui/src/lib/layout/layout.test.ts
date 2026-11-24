import { test } from 'node:test';
import assert from 'node:assert/strict';
import { arePanelsSuspended, pane, paneIds, splitPane } from './layout.ts';

test('suspends workspace panels on Home and Fleet Lead', () => {
  for (const kind of [undefined, null, 'fleet']) assert.equal(arePanelsSuspended(kind), true);
  for (const kind of ['local', 'ssh', 'quick', 'sftp', 'editor']) assert.equal(arePanelsSuspended(kind), false);
});

test('a split puts the new pane after the target, or before it on request', () => {
  const root = splitPane(pane('a'), 'a', 'row', 'b');
  assert.deepEqual(paneIds(root), ['a', 'b']);
  assert.deepEqual(paneIds(splitPane(root, 'b', 'column', 'c', true)), ['a', 'c', 'b']);
  assert.deepEqual(paneIds(splitPane(root, 'a', 'row', 'c', true)), ['c', 'a', 'b']);
});
