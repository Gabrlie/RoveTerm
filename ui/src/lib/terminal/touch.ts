export const FONT_SIZE_MIN = 8;
export const FONT_SIZE_MAX = 32;
export const LONG_PRESS_MS = 450;
export const TOUCH_MOVE_SLOP = 10;

const MAX_ALTERNATE_SCROLL_LINES = 10;
type Point = { x: number; y: number };

export function touchDistance(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

export function pinchFontSize(initialFontSize: number, initialDistance: number, currentDistance: number): number {
  const scale = initialDistance > 0 ? currentDistance / initialDistance : 1;
  return Math.min(FONT_SIZE_MAX, Math.max(FONT_SIZE_MIN, Math.round(initialFontSize * scale)));
}

/** Finger motion uses screen coordinates. An upward drag advances the viewport toward newer lines. */
export function accumulateScrollLines(
  remainderPx: number,
  deltaY: number,
  lineHeightPx: number,
): { lines: number; remainderPx: number } {
  if (!Number.isFinite(lineHeightPx) || lineHeightPx <= 0) return { lines: 0, remainderPx: 0 };
  const pixels = remainderPx - deltaY;
  const lines = Math.trunc(pixels / lineHeightPx) || 0;
  return { lines, remainderPx: pixels - lines * lineHeightPx };
}

export function alternateScrollSequence(lines: number): string {
  const count = Math.min(Math.trunc(Math.abs(lines)), MAX_ALTERNATE_SCROLL_LINES);
  return (lines < 0 ? '\x1b[A' : '\x1b[B').repeat(count);
}

export function exceededTouchSlop(startX: number, startY: number, x: number, y: number, slop = TOUCH_MOVE_SLOP): boolean {
  return touchDistance({ x: startX, y: startY }, { x, y }) > slop;
}
