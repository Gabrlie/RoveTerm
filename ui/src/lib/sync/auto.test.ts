import assert from 'node:assert/strict';
import { test } from 'node:test';
import { runAutoSync } from './auto.ts';

const ready = async () => ({ configured: true, ready: true });

test('a failed automatic round reports the cause once and the next tick can succeed', async () => {
  const failure = { code: 'sync.unauthorized' };
  const notices: unknown[] = [];
  let rounds = 0;
  const run = async () => {
    if (++rounds === 1) throw failure;
    return true;
  };
  await runAutoSync(ready, run, (error) => notices.push(error));
  await runAutoSync(ready, run, (error) => notices.push(error));
  assert.equal(rounds, 2);
  assert.deepEqual(notices, [failure]);
});

test('a failed readiness check reports its error without starting a round', async () => {
  const failure = new Error('status unavailable');
  const notices: unknown[] = [];
  await runAutoSync(async () => { throw failure; }, () => assert.fail('unexpected sync'), (error) => notices.push(error));
  assert.deepEqual(notices, [failure]);
});

test('unconfigured, locked and already-running sync stay quiet', async () => {
  for (const status of [{ configured: false, ready: false }, { configured: true, ready: false }]) {
    await runAutoSync(async () => status, () => assert.fail('unexpected sync'), () => assert.fail('unexpected notice'));
  }
  await runAutoSync(ready, async () => false, () => assert.fail('unexpected notice'));
});
