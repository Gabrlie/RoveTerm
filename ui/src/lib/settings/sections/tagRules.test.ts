import { test } from 'node:test';
import assert from 'node:assert/strict';
import { appendRules, removeRule } from './tagRules.ts';

test('rule entry trims and deduplicates comma-separated or pasted multiline rules', () => {
  assert.deepEqual(appendRules(['git'], ' git, ls\r\n cat\n\n ls, file name, ,'), ['git', 'ls', 'cat', 'file name']);
  assert.deepEqual(appendRules(['git'], '  \r\n, '), ['git']);
  assert.deepEqual(appendRules(['legacy,rule'], 'new'), ['legacy,rule', 'new']);
});

test('rule removal keeps other tags in order', () => {
  assert.deepEqual(removeRule(['a', 'b', 'a', 'c'], 'a'), ['b', 'c']);
  assert.deepEqual(removeRule(['a'], 'missing'), ['a']);
  assert.deepEqual(removeRule(['a'], 'a'), []);
});
