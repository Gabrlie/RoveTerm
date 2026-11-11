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

test('html stays text and markup characters inside words stay text', () => {
  const nodes = parseInline('a snake_case_name and <b>tag</b> `<a & b>`');
  assert.equal(nodes.map((node) => (node.kind === 'text' || node.kind === 'code' ? node.text : '')).join(''), 'a snake_case_name and <b>tag</b> <a & b>');
  assert.ok(!nodes.some((node) => node.kind === 'em' || node.kind === 'strong'));
});

test('tables keep their alignment and inline markup', () => {
  const [table] = parseMarkdown('| Disk | Use |\n|:-----|----:|\n| `/` | **80%** |\n| /home | ~~12%~~ |');
  assert.ok(table?.kind === 'table');
  assert.deepEqual(table.align, ['left', 'right']);
  assert.deepEqual(table.header[0], [{ kind: 'text', text: 'Disk' }]);
  assert.deepEqual(table.rows[0]?.[0], [{ kind: 'code', text: '/' }]);
  assert.deepEqual(table.rows[1]?.[1], [{ kind: 'del', children: [{ kind: 'text', text: '12%' }] }]);
});

test('nested and task lists, quotes with blocks, line breaks', () => {
  const [list, quote, paragraph] = parseMarkdown('- [x] done\n- [ ] todo\n  1. first\n  2. second\n\n> **Note**\n>\n> - inside\n\nline  \nbreak');
  assert.ok(list?.kind === 'list');
  assert.deepEqual(list.items.map((item) => item.task), [true, false]);
  const nested = list.items[1]?.blocks.find((block) => block.kind === 'list');
  assert.ok(nested?.kind === 'list' && nested.ordered && nested.items.length === 2);
  assert.ok(quote?.kind === 'quote');
  assert.deepEqual(quote.blocks.map((block) => block.kind), ['paragraph', 'list']);
  assert.ok(paragraph?.kind === 'paragraph' && paragraph.content.some((node) => node.kind === 'break'));
});
