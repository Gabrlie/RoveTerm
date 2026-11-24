import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { defaultWordWrap, draftViewMode, EDITOR_SYMBOLS, shouldAutoFocusEditor } from './editorTouch.ts';

test('the symbol bar uses the requested order and inserts a literal tab for Tab', () => {
  assert.deepEqual(EDITOR_SYMBOLS.map((symbol) => symbol.label), [
    'Tab', '{', '}', '[', ']', '(', ')', '"', "'", ':', ';', '=', '/', '-', '_', '|',
  ]);
  assert.deepEqual(EDITOR_SYMBOLS.map((symbol) => symbol.text), [
    '\t', '{', '}', '[', ']', '(', ')', '"', "'", ':', ';', '=', '/', '-', '_', '|',
  ]);
});

test('word wrap starts enabled on compact viewports only', () => {
  assert.equal(defaultWordWrap(true), true);
  assert.equal(defaultWordWrap(false), false);
});

test('automatic focus is reserved for fine pointers', () => {
  assert.equal(shouldAutoFocusEditor(true), false);
  assert.equal(shouldAutoFocusEditor(false), true);
});

test('wide draft views keep both panes with or without a selection', () => {
  assert.equal(draftViewMode(false, null), 'both');
  assert.equal(draftViewMode(false, 'draft-id'), 'both');
});

test('compact draft views switch from the list to the editor on selection', () => {
  assert.equal(draftViewMode(true, null), 'list');
  assert.equal(draftViewMode(true, 'draft-id'), 'editor');
  assert.equal(draftViewMode(true, ''), 'editor');
});
