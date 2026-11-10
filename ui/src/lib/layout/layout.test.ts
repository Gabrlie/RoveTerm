import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pane, paneIds, splitPane } from './layout.ts';

test('a split puts the new pane after the target, or before it on request', () => {
  const root = splitPane(pane('a'), 'a', 'row', 'b');
  assert.deepEqual(paneIds(root), ['a', 'b']);
  assert.deepEqual(paneIds(splitPane(root, 'b', 'column', 'c', true)), ['a', 'c', 'b']);
  assert.deepEqual(paneIds(splitPane(root, 'a', 'row', 'c', true)), ['c', 'a', 'b']);
});
