import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { applyTextChanges, editorText, lineEnding } from './textformat.ts';

test('preserves BOM and every untouched mixed line ending', () => {
  const raw = '\uFEFFone\r\ntwo\nthree\rfour';
  assert.equal(editorText(raw), 'one\ntwo\nthree\nfour');
  assert.equal(applyTextChanges(raw, [{ from: 4, to: 7, insert: 'second' }]), '\uFEFFone\r\nsecond\nthree\rfour');
  assert.equal(applyTextChanges(raw, []), raw);
});

test('new lines use the predominant ending without converting existing lines', () => {
  assert.equal(lineEnding('a\r\nb\r\nc\n'), '\r\n');
  assert.equal(applyTextChanges('a\r\nb', [{ from: 1, to: 1, insert: '\nnew' }]), 'a\r\nnew\r\nb');
});

test('applies ordered multiple changes and UTF-16 ranges', () => {
  assert.equal(applyTextChanges('😀a\r\nb\r\n', [
    { from: 2, to: 3, insert: 'A' }, { from: 4, to: 5, insert: 'B\nC' },
  ]), '😀A\r\nB\r\nC\r\n');
  assert.equal(applyTextChanges('\uFEFF', [{ from: 0, to: 0, insert: 'start\n' }]), '\uFEFFstart\n');
  const original = '\uFEFFa\r\nb';
  const cleared = applyTextChanges(original, [{ from: 0, to: 3, insert: '' }]);
  assert.equal(cleared, '\uFEFF');
  assert.equal(applyTextChanges(cleared, [{ from: 0, to: 0, insert: 'a\nb' }], lineEnding(original)), original);
});
