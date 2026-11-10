import { test } from 'node:test';
import assert from 'node:assert/strict';
import { menuAccelerator } from './accelerator.ts';

test('command bindings become menu accelerators', () => {
  assert.equal(menuAccelerator('Meta+T'), 'Cmd+T');
  assert.equal(menuAccelerator('Meta+Shift+I'), 'Cmd+Shift+I');
  assert.equal(menuAccelerator('Meta+,'), 'Cmd+Comma');
  assert.equal(menuAccelerator('Meta+]'), 'Cmd+BracketRight');
});

test('bindings without command stay with the terminal', () => {
  assert.equal(menuAccelerator('Ctrl+Tab'), null);
  assert.equal(menuAccelerator('Ctrl+Shift+T'), null);
  assert.equal(menuAccelerator('Meta+→'), null);
});
