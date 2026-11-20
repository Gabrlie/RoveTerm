import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { acceptExternalUpdate } from './externalstate.ts';
import type { ExternalEditUpdate } from '../ipc.ts';
const row = (revision: number, state: ExternalEditUpdate['state'] = 'watching'): ExternalEditUpdate => ({ id: 'a', label: 'host', path: '/etc/example.conf', local_path: '/local/example.conf', state, error: null, revision });
test('events win over a stale initial list and closed rows cannot be resurrected', () => {
  const rows: Record<string, ExternalEditUpdate> = {};
  const revisions: Record<string, number> = {};
  assert.equal(acceptExternalUpdate(rows, revisions, row(3, 'saved')), true);
  assert.equal(acceptExternalUpdate(rows, revisions, row(1)), false);
  assert.equal(rows.a?.state, 'saved');
  assert.equal(acceptExternalUpdate(rows, revisions, row(4, 'closed')), true);
  assert.equal(rows.a, undefined);
  assert.equal(acceptExternalUpdate(rows, revisions, row(4)), false);
  assert.equal(acceptExternalUpdate(rows, revisions, row(2)), false);
  assert.equal(rows.a, undefined);
});
test('invalid revisions are ignored', () => {
  assert.equal(acceptExternalUpdate({}, {}, row(NaN)), false);
  assert.equal(acceptExternalUpdate({}, {}, row(-1)), false);
});
