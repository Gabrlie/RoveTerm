import { test } from 'node:test';
import assert from 'node:assert/strict';
import { activity, newestFirst, progress } from './transferview.ts';
import type { TransferUpdate } from '../ipc.ts';

const row = (id: string, state: TransferUpdate['state'], done: number, total: number): TransferUpdate => ({
  id,
  tag: 'download',
  name: id,
  src_label: 'host',
  src_path: `/r/${id}`,
  dst_label: 'local',
  dst_path: `/l/${id}`,
  state,
  bytes_done: done,
  bytes_total: total,
  files_done: 0,
  files_total: 1,
  skipped: false,
  error: null,
});

test('transfers read as progress, newest first, and overall activity', () => {
  assert.equal(progress(row('a', 'running', 25, 100)), 25);
  assert.equal(progress(row('a', 'done', 0, 0)), 100);
  assert.deepEqual(
    newestFirst([row('t1', 'done', 1, 1), row('t3', 'running', 0, 1), row('t2', 'done', 1, 1)], 2).map((entry) => entry.id),
    ['t3', 't2'],
  );
  assert.deepEqual(activity([row('a', 'running', 50, 100), row('b', 'queued', 0, 300), row('c', 'done', 9, 9)]), { active: 2, percent: 12.5 });
  assert.deepEqual(activity([row('c', 'done', 9, 9)]), { active: 0, percent: null });
});
