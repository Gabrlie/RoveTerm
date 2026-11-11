import { test } from 'node:test';
import assert from 'node:assert/strict';
import { terminalCopies, terminalPastes, type ClipboardContext } from './claims.ts';

const base: ClipboardContext = { terminalSelection: '', pageSelection: '', focusInTerminal: true, focusInTextField: false };

test('copy belongs to the terminal only when it has the only selection', () => {
  assert.equal(terminalCopies({ ...base, terminalSelection: 'ls' }), true);
  assert.equal(terminalCopies(base), false, 'nothing selected: the system copies');
  assert.equal(terminalCopies({ ...base, terminalSelection: 'ls', pageSelection: 'an AI reply' }), false);
  assert.equal(terminalCopies({ ...base, terminalSelection: 'ls', focusInTextField: true }), false);
});

test('paste goes to the terminal with the focus', () => {
  assert.equal(terminalPastes(base), true);
  assert.equal(terminalPastes({ ...base, focusInTerminal: false }), false);
  assert.equal(terminalPastes({ ...base, focusInTextField: true }), false);
});
