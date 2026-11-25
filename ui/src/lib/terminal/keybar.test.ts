import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  addKeyBarItem, applyModifiers, builtinKeyMeta, BUILTIN_KEY_CATALOG, consumeModifier,
  cycleModifier, decodeCustomSequence, DEFAULT_KEYBAR_LAYOUT, encodeKeyItem, MAX_KEYBAR_ROW_ITEMS,
  migrateKeyBarV2ToV3, moveKeyBarItem, moveKeyBarItemAcrossRows, parseKeyBarLayout, removeKeyBarItem, serializeKeyBarLayout,
  type BuiltinKeyId, type KeyBarItem, type KeyBarLayout,
} from './keybar.ts';

const builtin = (id: BuiltinKeyId): KeyBarItem => ({ kind: 'builtin', id });
const custom = (sequence: string): KeyBarItem => ({ kind: 'custom', id: 'custom-1', label: 'My key', sequence });

test('modifiers toggle between off and armed and reset after consumption', () => {
  assert.equal(cycleModifier('off'), 'armed');
  assert.equal(cycleModifier('armed'), 'off');
  assert.equal(consumeModifier('off'), 'off');
  assert.equal(consumeModifier('armed'), 'off');
});

test('Ctrl encodes upper and lowercase ASCII letters and punctuation aliases', () => {
  for (let i = 0; i < 26; i++) {
    const letter = String.fromCharCode(0x61 + i);
    const expected = { data: String.fromCharCode(i + 1), ctrl: 'off', alt: 'off' };
    assert.deepEqual(applyModifiers(letter, 'armed', 'off'), expected);
    assert.deepEqual(applyModifiers(letter.toUpperCase(), 'armed', 'off'), expected);
  }
  const aliases = { '[': 27, '\\': 28, ']': 29, '^': 30, '6': 30, '_': 31, '-': 31, '@': 0, ' ': 0, '2': 0, '?': 127 };
  for (const [key, byte] of Object.entries(aliases)) {
    assert.deepEqual(applyModifiers(key, 'armed', 'off'), { data: String.fromCharCode(byte), ctrl: 'off', alt: 'off' });
  }
  assert.deepEqual(applyModifiers('1', 'armed', 'off'), { data: '1', ctrl: 'off', alt: 'off' });
});

test('Alt prefixes Ctrl output and empty input leaves modifiers ready', () => {
  assert.deepEqual(applyModifiers('c', 'armed', 'armed'), { data: '\x1b\x03', ctrl: 'off', alt: 'off' });
  assert.deepEqual(applyModifiers('z', 'off', 'armed'), { data: '\x1bz', ctrl: 'off', alt: 'off' });
  assert.deepEqual(applyModifiers(' ', 'armed', 'armed'), { data: '\x1b\x00', ctrl: 'off', alt: 'off' });
  assert.deepEqual(applyModifiers('', 'armed', 'armed'), { data: '', ctrl: 'armed', alt: 'armed' });
});

test('IME text passes through verbatim with modifiers off, and Ctrl never corrupts it', () => {
  const data = '你好🌏';
  assert.deepEqual(applyModifiers(data, 'off', 'off'), { data, ctrl: 'off', alt: 'off' });
  assert.deepEqual(applyModifiers(data, 'armed', 'off'), { data, ctrl: 'off', alt: 'off' });
  assert.deepEqual(applyModifiers('你', 'armed', 'off'), { data: '你', ctrl: 'off', alt: 'off' });
  assert.deepEqual(applyModifiers(data, 'armed', 'armed'), { data: '\x1b' + data, ctrl: 'off', alt: 'off' });
});

test('custom sequences decode hex, short escapes and caret tokens', () => {
  assert.equal(decodeCustomSequence('\\x1b[A\\e[Z\\r\\n\\t\\\\\\xAF'), '\x1b[A\x1b[Z\r\n\t\\\xaf');
  assert.equal(decodeCustomSequence('^A ^Z ^[ ^] ^\\ ^^ ^_'), '\x01 \x1a \x1b \x1d \x1c \x1e \x1f');
  assert.equal(decodeCustomSequence('text\\^C\\^Dend'), 'text\x03\x04end');
  assert.equal(decodeCustomSequence('你好 sudo  ^thing ^Cat ^c ^C^D \\z \\x1g'), '你好 sudo  ^thing ^Cat ^c ^C^D \\z \\x1g');
  assert.equal(decodeCustomSequence('\\\\^C'), '\\^C');
  assert.equal(decodeCustomSequence(''), '');
});

test('directional keys use CSI modifier bits and consume armed modifiers', () => {
  const directions = { up: 'A', down: 'B', right: 'C', left: 'D' } as const;
  for (const id of Object.keys(directions) as (keyof typeof directions)[]) {
    const key = builtin(id);
    const suffix = directions[id];
    assert.deepEqual(encodeKeyItem(key, 'off', 'off'), { data: '\x1b[' + suffix, ctrl: 'off', alt: 'off' });
    assert.deepEqual(encodeKeyItem(key, 'armed', 'off'), { data: '\x1b[1;5' + suffix, ctrl: 'off', alt: 'off' });
    assert.deepEqual(encodeKeyItem(key, 'off', 'armed'), { data: '\x1b[1;3' + suffix, ctrl: 'off', alt: 'off' });
    assert.deepEqual(encodeKeyItem(key, 'armed', 'armed'), { data: '\x1b[1;7' + suffix, ctrl: 'off', alt: 'off' });
  }
});

test('action and modifier keys have no sequence and direct control combos preserve modifiers', () => {
  const actions = ['ctrl', 'alt', 'paste', 'snippets', 'toggleExtra', 'keyboard', 'customize'] as const;
  for (const id of actions) assert.equal(encodeKeyItem(builtin(id), 'armed', 'armed'), null);
  const combos = { ctrlC: 3, ctrlD: 4, ctrlZ: 26, ctrlL: 12, ctrlR: 18, ctrlA: 1, ctrlE: 5, ctrlB: 2, ctrlW: 23 } as const;
  for (const id of Object.keys(combos) as (keyof typeof combos)[]) {
    assert.deepEqual(encodeKeyItem(builtin(id), 'armed', 'armed'), { data: String.fromCharCode(combos[id]), ctrl: 'armed', alt: 'armed' });
  }
});

test('function and navigation keys emit standard terminal sequences', () => {
  const functions = ['f1', 'f2', 'f3', 'f4', 'f5', 'f6', 'f7', 'f8', 'f9', 'f10', 'f11', 'f12'] as const;
  const expected = ['\x1bOP', '\x1bOQ', '\x1bOR', '\x1bOS', ...[15, 17, 18, 19, 20, 21, 23, 24].map((n) => '\x1b[' + n + '~')];
  assert.deepEqual(functions.map((id) => encodeKeyItem(builtin(id), 'off', 'off')?.data), expected);
  const navigation = {
    esc: '\x1b', tab: '\t', shiftTab: '\x1b[Z', home: '\x1b[H', end: '\x1b[F',
    pgUp: '\x1b[5~', pgDn: '\x1b[6~', enter: '\r', backspace: '\x7f', delete: '\x1b[3~',
  } as const;
  for (const id of Object.keys(navigation) as (keyof typeof navigation)[]) {
    assert.equal(encodeKeyItem(builtin(id), 'off', 'off')?.data, navigation[id]);
  }
});

test('symbols and decoded custom keys apply modifiers once', () => {
  assert.deepEqual(encodeKeyItem(builtin('backslash'), 'armed', 'off'), { data: '\x1c', ctrl: 'off', alt: 'off' });
  assert.deepEqual(encodeKeyItem(builtin('space'), 'off', 'armed'), { data: '\x1b ', ctrl: 'off', alt: 'off' });
  assert.deepEqual(encodeKeyItem(custom('\\x61'), 'armed', 'armed'), { data: '\x1b\x01', ctrl: 'off', alt: 'off' });
  assert.deepEqual(encodeKeyItem(custom('你好\\r'), 'off', 'off'), { data: '你好\r', ctrl: 'off', alt: 'off' });
});

test('the catalog and default rows include the expected labels and repeatable keys', () => {
  const primary = ['esc', 'slash', 'dash', 'home', 'up', 'end', 'pgUp'] as const;
  const secondary = ['tab', 'ctrl', 'alt', 'left', 'down', 'right', 'pgDn'] as const;
  assert.deepEqual(DEFAULT_KEYBAR_LAYOUT, { primary: primary.map(builtin), secondary: secondary.map(builtin), expanded: true });
  assert.deepEqual(primary.map((id) => builtinKeyMeta(id).label), ['ESC', '/', '-', 'HOME', '↑', 'END', 'PGUP']);
  assert.deepEqual(secondary.map((id) => builtinKeyMeta(id).label), ['TAB', 'CTRL', 'ALT', '←', '↓', '→', 'PGDN']);
  assert.equal(MAX_KEYBAR_ROW_ITEMS, 7);
  assert.equal(new Set(BUILTIN_KEY_CATALOG.map((key) => key.id)).size, BUILTIN_KEY_CATALOG.length);
  assert.equal(BUILTIN_KEY_CATALOG.some(({ id }) => id === 'toggleExtra' || id === 'customize'), false);
  assert.deepEqual(BUILTIN_KEY_CATALOG.filter((key) => key.repeatable).map((key) => key.id),
    ['left', 'down', 'up', 'right', 'pgUp', 'pgDn', 'backspace', 'delete']);
  assert.equal(builtinKeyMeta('f7').label, 'F7');
  assert.equal(builtinKeyMeta('ctrlC').category, 'combo');
});

test('layout persistence round-trips and preserves custom sequence whitespace', () => {
  for (const expanded of [true, false]) {
    const layout = { primary: [builtin('ctrl'), custom('sudo ')], secondary: [custom(' ')], expanded };
    const loaded = parseKeyBarLayout(serializeKeyBarLayout(layout));
    assert.deepEqual(loaded, layout);
    assert.notEqual(loaded.primary[0], layout.primary[0]);
  }
});

test('layout parsing filters invalid items, caps rows and defaults invalid primary layouts', () => {
  const unknown = { kind: 'builtin', id: 'does-not-exist' };
  const invalid = [null, 'esc', unknown, { kind: 'builtin', id: '__proto__' }, builtin('toggleExtra'), builtin('customize'),
    { ...custom('ls'), id: ' ' }, { ...custom('ls'), label: ' ' }, custom(''), { ...custom('ls'), sequence: 42 }];
  const layout = parseKeyBarLayout({
    primary: [...invalid, builtin('tab'), { ...custom('ls '), label: '  My key  ' }],
    secondary: [...invalid, ...Array.from({ length: 40 }, () => builtin('f1'))],
    expanded: 'true',
  });
  assert.deepEqual(layout.primary, [builtin('tab'), custom('ls ')]);
  assert.equal(layout.secondary.length, MAX_KEYBAR_ROW_ITEMS);
  assert.equal(layout.expanded, true);
  assert.deepEqual(parseKeyBarLayout({ primary: [builtin('tab')], secondary: 'bad' }), {
    primary: [builtin('tab')], secondary: [], expanded: true,
  });
  for (const value of [null, false, '[broken', {}, { primary: invalid, secondary: [builtin('ctrl')], expanded: true }]) {
    const fallback = parseKeyBarLayout(value);
    assert.deepEqual(fallback, DEFAULT_KEYBAR_LAYOUT);
    assert.notEqual(fallback.primary, DEFAULT_KEYBAR_LAYOUT.primary);
  }
});

const v1Default: KeyBarLayout = {
  primary: (['esc', 'tab', 'ctrl', 'alt', 'left', 'down', 'up', 'right', 'paste', 'snippets', 'toggleExtra', 'keyboard'] as const).map(builtin),
  secondary: (['home', 'end', 'pgUp', 'pgDn', 'slash', 'dash', 'pipe', 'tilde', 'underscore', 'colon', 'star', 'amp', 'backslash', 'customize'] as const).map(builtin),
  expanded: false,
};

const v2Default: KeyBarLayout = {
  primary: (['esc', 'tab', 'shiftTab', 'ctrl', 'alt', 'left', 'down', 'up', 'right', 'paste', 'snippets', 'keyboard'] as const).map(builtin),
  secondary: (['ctrlC', 'ctrlD', 'ctrlZ', 'home', 'end', 'pgUp', 'pgDn', 'slash', 'dash', 'pipe', 'tilde', 'underscore', 'colon', 'star', 'amp', 'backslash'] as const).map(builtin),
  expanded: true,
};

test('migration replaces legacy default rows with independent copies of the 2×7 default', () => {
  for (const layout of [v1Default, v2Default, { ...v2Default, expanded: false }]) {
    const migrated = migrateKeyBarV2ToV3(serializeKeyBarLayout(layout));
    assert.deepEqual(migrated, DEFAULT_KEYBAR_LAYOUT);
    assert.notEqual(migrated.primary[0], DEFAULT_KEYBAR_LAYOUT.primary[0]);
    assert.notEqual(migrated.secondary[0], DEFAULT_KEYBAR_LAYOUT.secondary[0]);
  }
});

test('migration removes pinned actions before capping each customized row at seven keys', () => {
  const userKey = { ...custom('sudo '), id: 'customize' };
  const primary = [builtin('esc'), userKey, builtin('keyboard'), ...v2Default.primary.slice(3, 7)];
  const secondary = [custom('你好\\r'), ...v2Default.secondary.slice(1, 7)];
  const layout = {
    primary: [builtin('toggleExtra'), ...primary, builtin('f7')],
    secondary: [builtin('customize'), ...secondary, builtin('f8')],
    expanded: false,
  };
  assert.deepEqual(migrateKeyBarV2ToV3(serializeKeyBarLayout(layout)), { primary, secondary, expanded: true });
  assert.deepEqual(migrateKeyBarV2ToV3({ ...layout, secondary: [] }).secondary, []);
});

test('migration detects customizations beyond the first seven keys', () => {
  const layout = { ...v2Default, primary: [...v2Default.primary.slice(0, -1), custom('sudo ')] };
  assert.deepEqual(migrateKeyBarV2ToV3(layout), {
    primary: v2Default.primary.slice(0, 7),
    secondary: v2Default.secondary.slice(0, 7),
    expanded: true,
  });
});

test('migration falls back to default keys when necessary and keeps a valid secondary row', () => {
  const fallback = migrateKeyBarV2ToV3('[broken');
  assert.deepEqual(fallback, DEFAULT_KEYBAR_LAYOUT);
  assert.notEqual(fallback.primary[0], DEFAULT_KEYBAR_LAYOUT.primary[0]);
  const layout = migrateKeyBarV2ToV3({ primary: [builtin('toggleExtra')], secondary: [custom('ls '), builtin('customize')] });
  assert.deepEqual(layout.primary, DEFAULT_KEYBAR_LAYOUT.primary);
  assert.deepEqual(layout.secondary, [custom('ls ')]);
  assert.deepEqual(parseKeyBarLayout(serializeKeyBarLayout(layout)), layout);
});

test('immutable editors reorder, move across rows, append and remove', () => {
  const initial: KeyBarLayout = { primary: [builtin('esc'), builtin('tab')], secondary: [builtin('f1')], expanded: true };
  const reordered = moveKeyBarItem(initial, 'primary', 1, -1);
  assert.deepEqual(reordered.primary, [builtin('tab'), builtin('esc')]);
  assert.deepEqual(moveKeyBarItem(reordered, 'primary', 0, 1), initial);
  const moved = moveKeyBarItemAcrossRows(reordered, 'primary', 0);
  assert.deepEqual(moved, { primary: [builtin('esc')], secondary: [builtin('f1'), builtin('tab')], expanded: true });
  assert.deepEqual(moveKeyBarItemAcrossRows(moved, 'secondary', 1), initial);
  const added = addKeyBarItem(moved, 'secondary', custom('^C'));
  assert.deepEqual(added.secondary, [builtin('f1'), builtin('tab'), custom('^C')]);
  assert.deepEqual(removeKeyBarItem(added, 'secondary', 2), moved);
  assert.deepEqual(removeKeyBarItem(initial, 'secondary', 0).secondary, []);
  assert.deepEqual(initial, { primary: [builtin('esc'), builtin('tab')], secondary: [builtin('f1')], expanded: true });
});

test('editors enforce row bounds, capacity and a nonempty primary row', () => {
  const layout: KeyBarLayout = {
    primary: [builtin('esc')],
    secondary: Array.from({ length: MAX_KEYBAR_ROW_ITEMS }, () => builtin('f2')),
    expanded: false,
  };
  assert.equal(removeKeyBarItem(layout, 'primary', 0), layout);
  assert.equal(removeKeyBarItem(layout, 'secondary', 0.5), layout);
  assert.equal(moveKeyBarItem(layout, 'primary', 0, -1), layout);
  assert.equal(moveKeyBarItem(layout, 'secondary', MAX_KEYBAR_ROW_ITEMS - 1, 1), layout);
  assert.equal(moveKeyBarItemAcrossRows(layout, 'primary', 0), layout);
  assert.equal(moveKeyBarItemAcrossRows({ ...layout, secondary: [] }, 'primary', 0).primary.length, 1);
  assert.equal(moveKeyBarItemAcrossRows(layout, 'secondary', -1), layout);
  assert.equal(addKeyBarItem(layout, 'secondary', builtin('f3')), layout);
  const fullPrimary = { ...layout, primary: [builtin('esc'), builtin('tab')] };
  assert.equal(moveKeyBarItemAcrossRows(fullPrimary, 'primary', 1), fullPrimary);
  const spare = removeKeyBarItem(fullPrimary, 'secondary', 0);
  assert.equal(addKeyBarItem(spare, 'secondary', builtin('f3')).secondary.length, 7);
  assert.equal(moveKeyBarItemAcrossRows(spare, 'primary', 1).secondary.length, 7);
});
