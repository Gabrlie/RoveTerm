import { subscribeImeBottom } from './insets';
import { COMPACT_BREAKPOINT, isCompactWidth, isKeyboardOpen } from './viewport';

const browser = typeof window === 'undefined' ? undefined : window;
const pointer = browser?.matchMedia?.('(pointer: coarse)');

let width = $state(browser?.innerWidth ?? COMPACT_BREAKPOINT);
let coarse = $state(pointer?.matches ?? false);
let imeBottom = $state(0);
const compact = $derived(isCompactWidth(width));
const keyboardOpen = $derived(isKeyboardOpen(imeBottom));

export const viewport = {
  get width() { return width; },
  get compact() { return compact; },
  get coarse() { return coarse; },
  get imeBottom() { return imeBottom; },
  get keyboardOpen() { return keyboardOpen; },
  setImeBottom(value: number): void {
    imeBottom = value;
  },
};

if (browser) {
  browser.addEventListener('resize', () => { width = browser.innerWidth; });
  pointer?.addEventListener?.('change', (event) => { coarse = event.matches; });
  subscribeImeBottom(viewport.setImeBottom);
}
