import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canSendPrompt, requestForSend } from './requests.ts';

test('retry preserves the original text and request ID', () => {
  const original = requestForSend(null, 'inspect hosts', false, () => 'request-1');
  const retry = requestForSend(original, 'replacement text', true, () => { throw new Error('a retry must not create an ID'); });
  assert.equal(retry, original);
  assert.deepEqual(retry, { id: 'request-1', text: 'inspect hosts' });
  const next = requestForSend(original, 'apply update', false, () => 'request-2');
  assert.deepEqual(next, { id: 'request-2', text: 'apply update' });
  assert.deepEqual(original, { id: 'request-1', text: 'inspect hosts' });
});

test('planner recovery blocks sends until explicit confirmation and never overrides active work', () => {
  assert.equal(canSendPrompt(false, false, true, false), false);
  assert.equal(canSendPrompt(false, false, true, true), true);
  assert.equal(canSendPrompt(true, false, true, true), false);
  assert.equal(canSendPrompt(false, true, true, true), false);
  assert.equal(canSendPrompt(false, false, false, false), true);
});
