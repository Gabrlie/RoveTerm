import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canSendPrompt, requestForSend } from './requests.ts';

test('retry preserves the original text, request ID and frozen authorization', () => {
  const original = requestForSend(null, 'inspect hosts', false, false, () => 'request-1');
  const retry = requestForSend(original, 'replacement text', true, true, () => { throw new Error('a retry must not create an ID'); });
  assert.equal(retry, original);
  assert.deepEqual(retry, { id: 'request-1', text: 'inspect hosts', allowChanges: false });
  const next = requestForSend(original, 'apply update', false, true, () => 'request-2');
  assert.deepEqual(next, { id: 'request-2', text: 'apply update', allowChanges: true });
  assert.deepEqual(original, { id: 'request-1', text: 'inspect hosts', allowChanges: false });
});

test('planner recovery blocks sends until explicit confirmation and never overrides active work', () => {
  assert.equal(canSendPrompt(false, false, true, false), false);
  assert.equal(canSendPrompt(false, false, true, true), true);
  assert.equal(canSendPrompt(true, false, true, true), false);
  assert.equal(canSendPrompt(false, true, true, true), false);
  assert.equal(canSendPrompt(false, false, false, false), true);
});
