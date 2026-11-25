import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { enterKeyHint, shouldSendOnEnter, type ComposerKeyEvent } from './composerTouch.ts';

const enter: ComposerKeyEvent = {
  key: 'Enter',
  shiftKey: false,
  ctrlKey: false,
  metaKey: false,
  isComposing: false,
};

test('desktop Enter sends unless Shift is held', () => {
  assert.equal(shouldSendOnEnter(enter, false), true);
  assert.equal(shouldSendOnEnter({ ...enter, ctrlKey: true }, false), true);
  assert.equal(shouldSendOnEnter({ ...enter, metaKey: true }, false), true);
  assert.equal(shouldSendOnEnter({ ...enter, shiftKey: true }, false), false);
  assert.equal(shouldSendOnEnter({ ...enter, shiftKey: true, ctrlKey: true }, false), false);
});

test('touch Enter sends with Ctrl or Meta', () => {
  assert.equal(shouldSendOnEnter(enter, true), false);
  assert.equal(shouldSendOnEnter({ ...enter, shiftKey: true }, true), false);
  assert.equal(shouldSendOnEnter({ ...enter, ctrlKey: true }, true), true);
  assert.equal(shouldSendOnEnter({ ...enter, metaKey: true }, true), true);
  assert.equal(shouldSendOnEnter({ ...enter, shiftKey: true, ctrlKey: true }, true), true);
});

test('Enter defers to the input method while composing', () => {
  assert.equal(shouldSendOnEnter({ ...enter, isComposing: true }, false), false);
  assert.equal(shouldSendOnEnter({ ...enter, isComposing: true, ctrlKey: true }, true), false);
  assert.equal(shouldSendOnEnter({ ...enter, isComposing: true, metaKey: true }, true), false);
});

test('other keys leave the draft in place', () => {
  const event = { ...enter, key: 'Tab', ctrlKey: true, metaKey: true };
  assert.equal(shouldSendOnEnter(event, false), false);
  assert.equal(shouldSendOnEnter(event, true), false);
});

test('the Enter key hint reflects the pointer type', () => {
  assert.equal(enterKeyHint(false), 'send');
  assert.equal(enterKeyHint(true), 'enter');
});
