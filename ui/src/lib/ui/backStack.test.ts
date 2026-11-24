import { beforeEach, test } from 'node:test';
import assert from 'node:assert/strict';
import { clearBackStack, handleBack, pushBackHandler } from './backStack.ts';

beforeEach(clearBackStack);

test('handles back in LIFO order and consumes each handler', () => {
  const calls: string[] = [];
  pushBackHandler(() => calls.push('first'));
  pushBackHandler(() => calls.push('second'));

  assert.equal(handleBack(), true);
  assert.equal(handleBack(), true);
  assert.deepEqual(calls, ['second', 'first']);
  assert.equal(handleBack(), false);
});

test('disposes only its own entry when called out of order or twice', () => {
  let calls = 0;
  const handler = () => { calls += 1; };
  const dispose = pushBackHandler(handler);
  pushBackHandler(handler);

  dispose();
  dispose();
  assert.equal(handleBack(), true);
  assert.equal(handleBack(), false);
  assert.equal(calls, 1);
});

test('a handler can dispose itself without removing the next entry', () => {
  const calls: string[] = [];
  pushBackHandler(() => calls.push('first'));
  const dispose = pushBackHandler(() => {
    dispose();
    calls.push('second');
  });

  assert.equal(handleBack(), true);
  assert.equal(handleBack(), true);
  assert.deepEqual(calls, ['second', 'first']);
});

test('returns false for an empty stack', () => {
  assert.equal(handleBack(), false);
});
