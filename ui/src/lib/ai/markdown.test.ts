import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseInline, parseMarkdown } from './markdown.ts';

test('parses blocks', () => {
  const blocks = parseMarkdown('# Plan\n\nRun `ls`\nthen check.\n\n- one\n- **two**\n\n```bash\nsudo systemctl restart nginx\n```\n> note\n\n---');
  assert.deepEqual(
    blocks.map((block) => block.kind),
    ['heading', 'paragraph', 'list', 'code', 'quote', 'rule'],
  );
  assert.deepEqual(blocks[3], { kind: 'code', lang: 'bash', text: 'sudo systemctl restart nginx' });
  assert.deepEqual(blocks[1], { kind: 'paragraph', content: [{ kind: 'text', text: 'Run ' }, { kind: 'code', text: 'ls' }, { kind: 'text', text: ' then check.' }] });
});

test('an unclosed fence runs to the end while streaming', () => {
  assert.deepEqual(parseMarkdown('```\nls -la'), [{ kind: 'code', lang: '', text: 'ls -la' }]);
});

test('links are kept only for safe schemes', () => {
  assert.deepEqual(parseInline('[docs](https://example.com) [x](javascript:void)'), [
    { kind: 'link', href: 'https://example.com', children: [{ kind: 'text', text: 'docs' }] },
    { kind: 'text', text: ' ' },
    { kind: 'text', text: 'x' },
  ]);
});

test('markup characters inside words stay text', () => {
  assert.deepEqual(parseInline('a snake_case_name and <b>tag</b>'), [{ kind: 'text', text: 'a snake_case_name and <b>tag</b>' }]);
  assert.deepEqual(parseInline('*em* and __strong__'), [
    { kind: 'em', children: [{ kind: 'text', text: 'em' }] },
    { kind: 'text', text: ' and ' },
    { kind: 'strong', children: [{ kind: 'text', text: 'strong' }] },
  ]);
});
