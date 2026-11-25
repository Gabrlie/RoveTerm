import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { initialCompactDetail, masterDetailView } from './masterDetail.ts';

test('wide layouts show both panes regardless of the selected view', () => {
  assert.equal(masterDetailView(false, false), 'both');
  assert.equal(masterDetailView(false, true), 'both');
});

test('compact layouts show either the list or the detail pane', () => {
  assert.equal(masterDetailView(true, false), 'list');
  assert.equal(masterDetailView(true, true), 'detail');
});

test('compact layouts open to the detail pane when given an explicit target', () => {
  assert.equal(initialCompactDetail(true), true);
  assert.equal(initialCompactDetail(false), false);
});
