// The actions table: the single source for keyboard shortcuts, menus and the command palette.
// App shortcuts use ⌘ on macOS and Ctrl+Shift elsewhere, so plain Ctrl+letter always reaches the shell.
import type { MessageKey } from './i18n';

export type ActionId =
  | 'tab.newLocal'
  | 'tab.close'
  | 'tab.next'
  | 'tab.prev'
  | 'pane.splitRight'
  | 'pane.splitDown'
  | 'pane.focusNext'
  | 'pane.focusPrev'
  | 'terminal.copy'
  | 'terminal.paste'
  | 'palette.open'
  | 'host.quickConnect'
  | 'host.new'
  | 'sidebar.toggle'
  | `tab.goto${1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9}`;

export interface Action {
  id: ActionId;
  label: MessageKey;
  /** Default bindings. `App` means ⌘ on macOS and Ctrl+Shift elsewhere. */
  keys: string[];
  /** Hidden from the command palette. */
  hidden?: boolean;
}

export const isMac = /Mac|iPhone|iPad/.test(navigator.platform) || navigator.userAgent.includes('Mac OS');

export const actions: Action[] = [
  { id: 'tab.newLocal', label: 'action.tab.newLocal', keys: ['App+T'] },
  { id: 'host.quickConnect', label: 'action.host.quickConnect', keys: ['App+K'] },
  { id: 'host.new', label: 'action.host.new', keys: ['App+N'] },
  { id: 'sidebar.toggle', label: 'action.sidebar.toggle', keys: ['App+B'] },
  { id: 'tab.close', label: 'action.tab.close', keys: ['App+W'] },
  { id: 'tab.next', label: 'action.tab.next', keys: ['Ctrl+Tab'] },
  { id: 'tab.prev', label: 'action.tab.prev', keys: ['Ctrl+Shift+Tab'] },
  { id: 'pane.splitRight', label: 'action.pane.splitRight', keys: ['App+D'] },
  { id: 'pane.splitDown', label: 'action.pane.splitDown', keys: ['App+E'] },
  { id: 'pane.focusNext', label: 'action.pane.focusNext', keys: ['App+]'] },
  { id: 'pane.focusPrev', label: 'action.pane.focusPrev', keys: ['App+['] },
  { id: 'terminal.copy', label: 'action.terminal.copy', keys: ['App+C'] },
  { id: 'terminal.paste', label: 'action.terminal.paste', keys: ['App+V'] },
  { id: 'palette.open', label: 'action.palette.open', keys: ['App+P'] },
  ...([1, 2, 3, 4, 5, 6, 7, 8, 9] as const).map(
    (n): Action => ({ id: `tab.goto${n}`, label: 'action.tab.goto', keys: [`App+${n}`], hidden: true }),
  ),
];

const MODIFIER_ORDER = ['Ctrl', 'Alt', 'Shift', 'Meta'] as const;

/** Expands `App` and orders modifiers so bindings compare as plain strings. */
export function normalizeBinding(binding: string): string {
  const parts = binding.split('+').flatMap((part) => (part === 'App' ? (isMac ? ['Meta'] : ['Ctrl', 'Shift']) : [part]));
  const key = parts.filter((part) => !(MODIFIER_ORDER as readonly string[]).includes(part)).join('+');
  const modifiers = MODIFIER_ORDER.filter((modifier) => parts.includes(modifier));
  return [...modifiers, key.length === 1 ? key.toUpperCase() : key].join('+');
}

const CODE_KEYS: Record<string, string> = {
  BracketLeft: '[',
  BracketRight: ']',
  Comma: ',',
  Period: '.',
  Slash: '/',
  Semicolon: ';',
  Minus: '-',
  Equal: '=',
};

/** Describes a key event in binding syntax, using the physical key so layouts and Shift agree. */
export function eventBinding(event: KeyboardEvent): string {
  const { code } = event;
  let key: string;
  if (code.startsWith('Key')) key = code.slice(3);
  else if (code.startsWith('Digit')) key = code.slice(5);
  else key = CODE_KEYS[code] ?? event.key;
  const modifiers = MODIFIER_ORDER.filter((modifier) => event.getModifierState(modifier === 'Ctrl' ? 'Control' : modifier));
  return [...modifiers, key.length === 1 ? key.toUpperCase() : key].join('+');
}

let overrides: Partial<Record<ActionId, string[]>> = {};

/** Replaces the user's custom bindings. */
export function setOverrides(next: Partial<Record<ActionId, string[]>>): void {
  overrides = next;
  index = buildIndex();
}

export function bindingsOf(action: Action): string[] {
  return (overrides[action.id] ?? action.keys).map(normalizeBinding);
}

function buildIndex(): Map<string, ActionId> {
  const map = new Map<string, ActionId>();
  for (const action of actions) for (const binding of bindingsOf(action)) map.set(binding, action.id);
  return map;
}

let index = buildIndex();

/** Returns the action bound to `event`, if any. */
export function actionFor(event: KeyboardEvent): ActionId | undefined {
  return index.get(eventBinding(event));
}

/** `true` when the terminal must not receive `event` because the app handles it. */
export function isAppShortcut(event: KeyboardEvent): boolean {
  return actionFor(event) !== undefined;
}

/** Formats a binding for display, for example `⌘D` or `Ctrl+Shift+D`. */
export function displayBinding(binding: string): string {
  if (!isMac) return binding;
  const symbols: Record<string, string> = { Meta: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧' };
  return binding
    .split('+')
    .map((part) => symbols[part] ?? part)
    .join('');
}
