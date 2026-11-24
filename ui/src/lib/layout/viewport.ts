export const COMPACT_BREAKPOINT = 768;
export const KEYBOARD_OPEN_THRESHOLD = 40;

export function isCompactWidth(width: number): boolean {
  return width < COMPACT_BREAKPOINT;
}

export function isKeyboardOpen(imeBottom: number): boolean {
  return Number.isFinite(imeBottom) && imeBottom > KEYBOARD_OPEN_THRESHOLD;
}
