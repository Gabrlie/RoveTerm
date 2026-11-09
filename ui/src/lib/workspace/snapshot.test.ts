// Run with `pnpm test:ui` (Node's test runner with type stripping); not part of the app build.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseSnapshot } from './snapshot.ts';

test('a version 1 snapshot becomes one group of every pane, showing the old focus', () => {
  const parsed = parseSnapshot({
    version: 1,
    active: 1,
    tabs: [
      { root: { pane: { kind: 'local' } }, focused: 0 },
      {
        root: {
          dir: 'row',
          ratio: 0.5,
          a: { pane: { kind: 'ssh', hostId: 'h1', title: 'web' } },
          b: { pane: { kind: 'sftp', source: { hostId: 'h1' }, title: 'web' } },
        },
        focused: 1,
      },
    ],
  });
  assert.ok(parsed && 'group' in parsed.root);
  assert.deepEqual(
    parsed.root.group.tabs.map((tab) => tab.kind),
    ['local', 'ssh', 'sftp'],
  );
  assert.equal(parsed.root.group.active, 2);
});

test('a version 2 snapshot keeps splits and drops malformed tabs', () => {
  const parsed = parseSnapshot({
    version: 2,
    focused: 1,
    root: {
      dir: 'row',
      ratio: 0.3,
      a: { group: { tabs: [{ kind: 'local' }], active: 0 } },
      b: { group: { tabs: [{ kind: 'quick', address: 'a@b' }, { kind: 'bogus' }], active: 5 } },
    },
  });
  assert.ok(parsed && 'dir' in parsed.root && 'group' in parsed.root.b);
  assert.equal(parsed.root.ratio, 0.3);
  assert.equal(parsed.root.b.group.tabs.length, 1);
  assert.equal(parsed.root.b.group.active, -1);
  assert.equal(parsed.focused, 1);
});

test('unknown versions are ignored and the tab count is capped', () => {
  assert.equal(parseSnapshot({ version: 9 }), null);
  assert.equal(parseSnapshot('nonsense'), null);
  const many = parseSnapshot({
    version: 2,
    focused: 0,
    root: { group: { tabs: Array.from({ length: 100 }, () => ({ kind: 'local' })), active: 0 } },
  });
  assert.ok(many && 'group' in many.root);
  assert.equal(many.root.group.tabs.length, 64);
});
