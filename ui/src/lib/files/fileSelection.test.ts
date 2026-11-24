import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import {
  allSelected,
  exceededMoveThreshold,
  formatCompactMeta,
  LONG_PRESS_MOVE_PX,
  LONG_PRESS_MS,
  toggleSelectAll,
  toggleSelection,
  touchRowAction,
} from './fileSelection.ts';
import { formatTime } from './format.ts';

test('touch opens directories and symbolic links and edits recognized text files', () => {
  assert.equal(touchRowAction({ kind: 'dir', name: 'project', size: 0 }), 'open');
  assert.equal(touchRowAction({ kind: 'symlink', name: 'shortcut.txt', size: 10 }), 'open');
  assert.equal(touchRowAction({ kind: 'file', name: 'README.MD', size: 2 * 1024 * 1024 }), 'edit');
});

test('touch offers actions for binary, oversized, and unrecognized files', () => {
  assert.equal(touchRowAction({ kind: 'file', name: 'archive.zip', size: 1024 }), 'menu');
  assert.equal(touchRowAction({ kind: 'file', name: 'large.txt', size: 2 * 1024 * 1024 + 1 }), 'menu');
  assert.equal(touchRowAction({ kind: 'file', name: 'unknown', size: 0 }), 'menu');
  assert.equal(touchRowAction({ kind: 'other', name: 'socket.txt', size: 0 }), 'menu');
});

test('selection toggles a name without changing the previous set', () => {
  const selected = new Set(['a']);
  assert.deepEqual(toggleSelection(selected, 'b'), new Set(['a', 'b']));
  assert.deepEqual(toggleSelection(selected, 'a'), new Set());
  assert.deepEqual(selected, new Set(['a']));
});

test('select all uses the shown names and clears when all are selected', () => {
  const shown = ['a', 'b'];
  const partial = new Set(['a', '.hidden']);
  const full = new Set(['a', 'b', '.hidden']);
  assert.equal(allSelected(shown, partial), false);
  assert.equal(allSelected(shown, full), true);
  assert.equal(allSelected([], full), false);
  assert.deepEqual(toggleSelectAll(shown, partial), new Set(shown));
  assert.deepEqual(toggleSelectAll(shown, full), new Set());
  assert.deepEqual(toggleSelectAll([], full), new Set());
});

test('long press cancels only when Euclidean movement exceeds the threshold', () => {
  assert.equal(LONG_PRESS_MS, 450);
  assert.equal(LONG_PRESS_MOVE_PX, 10);
  assert.equal(exceededMoveThreshold(20, 20, 20, 20), false);
  assert.equal(exceededMoveThreshold(20, 20, 14, 28), false);
  assert.equal(exceededMoveThreshold(20, 20, 12, 28), true);
  assert.equal(exceededMoveThreshold(20, 20, 12, 28, 12), false);
});

test('compact metadata lists size, time and permissions and omits missing fields', () => {
  const modified = 0;
  assert.equal(
    formatCompactMeta({ kind: 'file', size: 1536, modified, permissions: 0o640 }),
    `1.5 KB · ${formatTime(modified)} · rw-r-----`,
  );
  assert.equal(formatCompactMeta({ kind: 'dir', size: 4096, modified: null, permissions: 0o755 }), 'rwxr-xr-x');
  assert.equal(formatCompactMeta({ kind: 'dir', size: 4096, modified: null, permissions: null }), '');
  assert.equal(formatCompactMeta({ kind: 'file', size: 0, modified: null, permissions: null }), '0 B');
});
