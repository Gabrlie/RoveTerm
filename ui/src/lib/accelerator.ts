// macOS menu key equivalents from app bindings.

const KEY_NAMES: Record<string, string> = {
  ',': 'Comma',
  '.': 'Period',
  '/': 'Slash',
  ';': 'Semicolon',
  '-': 'Minus',
  '=': 'Equal',
  '[': 'BracketLeft',
  ']': 'BracketRight',
};

const MODIFIERS: Record<string, string> = { Meta: 'Cmd', Ctrl: 'Ctrl', Alt: 'Alt', Shift: 'Shift' };

/**
 * The menu accelerator for a normalized binding (`Meta+Shift+I` → `Cmd+Shift+I`), or `null` when
 * the binding has no ⌘: those reach the terminal, so the menu must not take them.
 */
export function menuAccelerator(binding: string): string | null {
  const parts = binding.split('+').filter((part) => part.length > 0);
  if (!parts.includes('Meta')) return null;
  const converted = parts.map((part) => MODIFIERS[part] ?? KEY_NAMES[part] ?? part);
  return converted.every((part) => /^[A-Za-z0-9]+$/.test(part)) ? converted.join('+') : null;
}
