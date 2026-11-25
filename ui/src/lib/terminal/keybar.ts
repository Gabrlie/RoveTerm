export type ModifierState = 'off' | 'armed';

export function cycleModifier(state: ModifierState): ModifierState {
  return state === 'off' ? 'armed' : 'off';
}

export function consumeModifier(state: ModifierState): ModifierState {
  return state === 'armed' ? 'off' : state;
}

const CTRL_CHARACTERS: Readonly<Record<string, string>> = {
  '[': '\x1b', '\\': '\x1c', ']': '\x1d', '^': '\x1e', '6': '\x1e',
  '_': '\x1f', '-': '\x1f', '@': '\x00', ' ': '\x00', '2': '\x00', '?': '\x7f',
};

function controlCharacter(data: string): string {
  if (data.length !== 1) return data;
  if (/[a-zA-Z]/.test(data)) return String.fromCharCode(data.charCodeAt(0) & 0x1f);
  return CTRL_CHARACTERS[data] ?? data;
}

export function applyModifiers(data: string, ctrl: ModifierState, alt: ModifierState): ModifiedInput {
  if (!data || (ctrl === 'off' && alt === 'off')) return { data, ctrl, alt };
  const text = ctrl === 'off' ? data : controlCharacter(data);
  return {
    data: alt === 'off' ? text : '\x1b' + text,
    ctrl: consumeModifier(ctrl),
    alt: consumeModifier(alt),
  };
}

interface ModifiedInput {
  data: string;
  ctrl: ModifierState;
  alt: ModifierState;
}

export type BuiltinKeyId =
  | 'esc' | 'tab' | 'shiftTab' | 'ctrl' | 'alt' | 'left' | 'down' | 'up' | 'right'
  | 'home' | 'end' | 'pgUp' | 'pgDn' | 'enter' | 'backspace' | 'delete' | 'space'
  | 'f1' | 'f2' | 'f3' | 'f4' | 'f5' | 'f6' | 'f7' | 'f8' | 'f9' | 'f10' | 'f11' | 'f12'
  | 'slash' | 'dash' | 'pipe' | 'tilde' | 'underscore' | 'colon' | 'star' | 'amp'
  | 'backslash' | 'dollar' | 'squote' | 'dquote' | 'backtick' | 'lbracket' | 'rbracket'
  | 'lbrace' | 'rbrace' | 'lt' | 'gt' | 'ctrlC' | 'ctrlD' | 'ctrlZ' | 'ctrlL'
  | 'ctrlR' | 'ctrlA' | 'ctrlE' | 'ctrlB' | 'ctrlW' | 'paste' | 'snippets'
  | 'toggleExtra' | 'keyboard' | 'customize';

export type KeyBarItem =
  | { kind: 'builtin'; id: BuiltinKeyId }
  | { kind: 'custom'; id: string; label: string; sequence: string };

export interface KeyBarLayout {
  primary: KeyBarItem[];
  secondary: KeyBarItem[];
  expanded: boolean;
}

export interface BuiltinKeyMeta {
  id: BuiltinKeyId;
  label: string;
  category: 'modifier' | 'nav' | 'function' | 'symbol' | 'combo' | 'action';
  repeatable?: boolean;
}

interface KeyDefinition extends Omit<BuiltinKeyMeta, 'id'> {
  sequence?: string;
  direction?: 'A' | 'B' | 'C' | 'D';
}

const BUILTIN_KEYS: Record<BuiltinKeyId, KeyDefinition> = {
  ctrl: { label: 'CTRL', category: 'modifier' },
  alt: { label: 'ALT', category: 'modifier' },
  esc: { label: 'ESC', category: 'nav', sequence: '\x1b' },
  tab: { label: 'TAB', category: 'nav', sequence: '\t' },
  shiftTab: { label: 'Shift+Tab', category: 'nav', sequence: '\x1b[Z' },
  left: { label: '←', category: 'nav', direction: 'D', repeatable: true },
  down: { label: '↓', category: 'nav', direction: 'B', repeatable: true },
  up: { label: '↑', category: 'nav', direction: 'A', repeatable: true },
  right: { label: '→', category: 'nav', direction: 'C', repeatable: true },
  home: { label: 'HOME', category: 'nav', sequence: '\x1b[H' },
  end: { label: 'END', category: 'nav', sequence: '\x1b[F' },
  pgUp: { label: 'PGUP', category: 'nav', sequence: '\x1b[5~', repeatable: true },
  pgDn: { label: 'PGDN', category: 'nav', sequence: '\x1b[6~', repeatable: true },
  enter: { label: 'Enter', category: 'nav', sequence: '\r' },
  backspace: { label: '⌫', category: 'nav', sequence: '\x7f', repeatable: true },
  delete: { label: 'Del', category: 'nav', sequence: '\x1b[3~', repeatable: true },
  space: { label: '␣', category: 'symbol', sequence: ' ' },
  slash: { label: '/', category: 'symbol', sequence: '/' },
  dash: { label: '-', category: 'symbol', sequence: '-' },
  pipe: { label: '|', category: 'symbol', sequence: '|' },
  tilde: { label: '~', category: 'symbol', sequence: '~' },
  underscore: { label: '_', category: 'symbol', sequence: '_' },
  colon: { label: ':', category: 'symbol', sequence: ':' },
  star: { label: '*', category: 'symbol', sequence: '*' },
  amp: { label: '&', category: 'symbol', sequence: '&' },
  backslash: { label: '\\', category: 'symbol', sequence: '\\' },
  dollar: { label: '$', category: 'symbol', sequence: '$' },
  squote: { label: "'", category: 'symbol', sequence: "'" },
  dquote: { label: '"', category: 'symbol', sequence: '"' },
  backtick: { label: '\x60', category: 'symbol', sequence: '\x60' },
  lbracket: { label: '[', category: 'symbol', sequence: '[' },
  rbracket: { label: ']', category: 'symbol', sequence: ']' },
  lbrace: { label: '{', category: 'symbol', sequence: '{' },
  rbrace: { label: '}', category: 'symbol', sequence: '}' },
  lt: { label: '<', category: 'symbol', sequence: '<' },
  gt: { label: '>', category: 'symbol', sequence: '>' },
  f1: { label: 'F1', category: 'function', sequence: '\x1bOP' },
  f2: { label: 'F2', category: 'function', sequence: '\x1bOQ' },
  f3: { label: 'F3', category: 'function', sequence: '\x1bOR' },
  f4: { label: 'F4', category: 'function', sequence: '\x1bOS' },
  f5: { label: 'F5', category: 'function', sequence: '\x1b[15~' },
  f6: { label: 'F6', category: 'function', sequence: '\x1b[17~' },
  f7: { label: 'F7', category: 'function', sequence: '\x1b[18~' },
  f8: { label: 'F8', category: 'function', sequence: '\x1b[19~' },
  f9: { label: 'F9', category: 'function', sequence: '\x1b[20~' },
  f10: { label: 'F10', category: 'function', sequence: '\x1b[21~' },
  f11: { label: 'F11', category: 'function', sequence: '\x1b[23~' },
  f12: { label: 'F12', category: 'function', sequence: '\x1b[24~' },
  ctrlC: { label: 'Ctrl+C', category: 'combo', sequence: '\x03' },
  ctrlD: { label: 'Ctrl+D', category: 'combo', sequence: '\x04' },
  ctrlZ: { label: 'Ctrl+Z', category: 'combo', sequence: '\x1a' },
  ctrlL: { label: 'Ctrl+L', category: 'combo', sequence: '\x0c' },
  ctrlR: { label: 'Ctrl+R', category: 'combo', sequence: '\x12' },
  ctrlA: { label: 'Ctrl+A', category: 'combo', sequence: '\x01' },
  ctrlE: { label: 'Ctrl+E', category: 'combo', sequence: '\x05' },
  ctrlB: { label: 'Ctrl+B', category: 'combo', sequence: '\x02' },
  ctrlW: { label: 'Ctrl+W', category: 'combo', sequence: '\x17' },
  paste: { label: 'Paste', category: 'action' },
  snippets: { label: 'Snippets', category: 'action' },
  toggleExtra: { label: 'More', category: 'action' },
  keyboard: { label: 'Keyboard', category: 'action' },
  customize: { label: 'Settings', category: 'action' },
};

export function builtinKeyMeta(id: BuiltinKeyId): BuiltinKeyMeta {
  const { label, category, repeatable } = BUILTIN_KEYS[id];
  return { id, label, category, repeatable };
}

export function isPinnedKey(id: BuiltinKeyId): boolean {
  return id === 'toggleExtra' || id === 'customize';
}

export const BUILTIN_KEY_CATALOG: readonly BuiltinKeyMeta[] =
  (Object.keys(BUILTIN_KEYS) as BuiltinKeyId[]).filter((id) => !isPinnedKey(id)).map(builtinKeyMeta);

export const DEFAULT_KEYBAR_LAYOUT: KeyBarLayout = {
  primary: builtinRow('esc', 'slash', 'dash', 'home', 'up', 'end', 'pgUp'),
  secondary: builtinRow('tab', 'ctrl', 'alt', 'left', 'down', 'right', 'pgDn'),
  expanded: true,
};

function builtinRow(...ids: BuiltinKeyId[]): KeyBarItem[] {
  return ids.map((id) => ({ kind: 'builtin', id }));
}

const ESCAPED_CHARACTERS: Readonly<Record<string, string>> = {
  e: '\x1b', r: '\r', n: '\n', t: '\t', '\\': '\\',
};

/** Caret tokens are whitespace-delimited. A backslash marks an embedded caret escape. */
export function decodeCustomSequence(raw: string): string {
  return raw.replace(
    /\\x([\da-fA-F]{2})|\\([ernt\\])|(\\?)(\^[A-Z[\]\\^_])/g,
    (token, hex: string | undefined, escaped: string | undefined, backslash: string, caret: string, offset: number) => {
      if (hex) return String.fromCharCode(Number.parseInt(hex, 16));
      if (escaped) return ESCAPED_CHARACTERS[escaped] ?? token;
      const before = raw[offset - 1] ?? ' ';
      const after = raw[offset + token.length] ?? ' ';
      if (backslash || (/\s/.test(before) && /\s/.test(after))) return controlCharacter(caret.charAt(1));
      return token;
    },
  );
}

export function encodeKeyItem(item: KeyBarItem, ctrl: ModifierState, alt: ModifierState): ModifiedInput | null {
  if (item.kind === 'custom') return applyModifiers(decodeCustomSequence(item.sequence), ctrl, alt);
  const key = BUILTIN_KEYS[item.id];
  if (key.direction) {
    const modifier = 1 + (ctrl === 'off' ? 0 : 4) + (alt === 'off' ? 0 : 2);
    const prefix = modifier === 1 ? '\x1b[' : '\x1b[1;' + modifier;
    return { data: prefix + key.direction, ctrl: consumeModifier(ctrl), alt: consumeModifier(alt) };
  }
  if (key.sequence === undefined) return null;
  if (key.category === 'combo') return { data: key.sequence, ctrl, alt };
  return applyModifiers(key.sequence, ctrl, alt);
}

export const MAX_KEYBAR_ROW_ITEMS = 7;
export type KeyBarRow = 'primary' | 'secondary';

function parseRow(raw: unknown, limit = MAX_KEYBAR_ROW_ITEMS): KeyBarItem[] {
  if (!Array.isArray(raw)) return [];
  const row: KeyBarItem[] = [];
  for (const value of raw as unknown[]) {
    if (row.length === limit) break;
    if (typeof value !== 'object' || value === null) continue;
    const item = value as Partial<Record<'kind' | 'id' | 'label' | 'sequence', unknown>>;
    if (typeof item.id !== 'string') continue;
    if (item.kind === 'builtin' && Object.hasOwn(BUILTIN_KEYS, item.id)) {
      const id = item.id as BuiltinKeyId;
      if (!isPinnedKey(id)) row.push({ kind: 'builtin', id });
    } else if (
      item.kind === 'custom' && item.id.trim() &&
      typeof item.label === 'string' && item.label.trim() &&
      typeof item.sequence === 'string' && item.sequence.length > 0
    ) {
      row.push({ kind: 'custom', id: item.id, label: item.label.trim(), sequence: item.sequence });
    }
  }
  return row;
}

function readLayout(raw: unknown): Partial<Record<keyof KeyBarLayout, unknown>> {
  if (typeof raw === 'string') {
    try {
      raw = JSON.parse(raw);
    } catch {
      return {};
    }
  }
  return typeof raw === 'object' && raw !== null ? raw : {};
}

export function parseKeyBarLayout(raw: unknown): KeyBarLayout {
  const layout = readLayout(raw);
  const primary = parseRow(layout.primary);
  if (!primary.length) return structuredClone(DEFAULT_KEYBAR_LAYOUT);
  return {
    primary,
    secondary: parseRow(layout.secondary),
    expanded: typeof layout.expanded === 'boolean' ? layout.expanded : DEFAULT_KEYBAR_LAYOUT.expanded,
  };
}

const LEGACY_DEFAULT_ROWS: readonly Record<KeyBarRow, readonly BuiltinKeyId[]>[] = [
  {
    primary: ['esc', 'tab', 'ctrl', 'alt', 'left', 'down', 'up', 'right', 'paste', 'snippets', 'keyboard'],
    secondary: ['home', 'end', 'pgUp', 'pgDn', 'slash', 'dash', 'pipe', 'tilde', 'underscore', 'colon', 'star', 'amp', 'backslash'],
  },
  {
    primary: ['esc', 'tab', 'shiftTab', 'ctrl', 'alt', 'left', 'down', 'up', 'right', 'paste', 'snippets', 'keyboard'],
    secondary: ['ctrlC', 'ctrlD', 'ctrlZ', 'home', 'end', 'pgUp', 'pgDn', 'slash', 'dash', 'pipe', 'tilde', 'underscore', 'colon', 'star', 'amp', 'backslash'],
  },
];

function matchesBuiltinRow(row: KeyBarItem[], ids: readonly BuiltinKeyId[]): boolean {
  return row.length === ids.length && row.every((item, index) => item.kind === 'builtin' && item.id === ids[index]);
}

/** Upgrades saved v1 or v2 layouts, comparing the full key order before applying the v3 cap. */
export function migrateKeyBarV2ToV3(rawV2: unknown): KeyBarLayout {
  const layout = readLayout(rawV2);
  const primary = parseRow(layout.primary, Infinity);
  const secondary = parseRow(layout.secondary, Infinity);
  if (
    (!primary.length && !secondary.length) ||
    LEGACY_DEFAULT_ROWS.some((defaults) => matchesBuiltinRow(primary, defaults.primary) && matchesBuiltinRow(secondary, defaults.secondary))
  ) {
    return structuredClone(DEFAULT_KEYBAR_LAYOUT);
  }
  return {
    primary: primary.length ? primary.slice(0, MAX_KEYBAR_ROW_ITEMS) : structuredClone(DEFAULT_KEYBAR_LAYOUT.primary),
    secondary: secondary.slice(0, MAX_KEYBAR_ROW_ITEMS),
    expanded: true,
  };
}

export function serializeKeyBarLayout(layout: KeyBarLayout): string {
  return JSON.stringify(layout);
}

export function moveKeyBarItem(layout: KeyBarLayout, row: KeyBarRow, index: number, delta: -1 | 1): KeyBarLayout {
  const items = layout[row];
  const item = items[index];
  const neighbor = items[index + delta];
  if (!item || !neighbor) return layout;
  const next = items.slice();
  [next[index], next[index + delta]] = [neighbor, item];
  return { ...layout, [row]: next };
}

export function removeKeyBarItem(layout: KeyBarLayout, row: KeyBarRow, index: number): KeyBarLayout {
  const items = layout[row];
  if (!items[index] || (row === 'primary' && items.length === 1)) return layout;
  return { ...layout, [row]: items.filter((_, position) => position !== index) };
}

export function addKeyBarItem(layout: KeyBarLayout, row: KeyBarRow, item: KeyBarItem): KeyBarLayout {
  if (layout[row].length >= MAX_KEYBAR_ROW_ITEMS) return layout;
  return { ...layout, [row]: [...layout[row], item] };
}

export function moveKeyBarItemAcrossRows(layout: KeyBarLayout, fromRow: KeyBarRow, index: number): KeyBarLayout {
  const toRow = fromRow === 'primary' ? 'secondary' : 'primary';
  const item = layout[fromRow][index];
  if (!item || layout[toRow].length >= MAX_KEYBAR_ROW_ITEMS) return layout;
  const next = removeKeyBarItem(layout, fromRow, index);
  return next === layout ? layout : addKeyBarItem(next, toRow, item);
}
