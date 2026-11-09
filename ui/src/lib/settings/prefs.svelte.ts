// User preferences shared by the UI. Loaded once at startup; views read them reactively.
import { prefsGet, prefsSet, type Preferences } from '../ipc';

/** Family of the bundled symbol font (see styles/fonts.css). */
export const SYMBOL_FONT = 'Symbols Nerd Font Mono';

const DEFAULTS: Preferences = { terminal: { font_family: '', font_size: 13, line_height: 1.15 } };

/**
 * CSS font list for the terminal: `family` when set, then the built-in stack from the `--font-mono`
 * token (ending in the generic `monospace`), then the bundled symbol font for Powerline and Nerd
 * Font icons. The symbol font stays last: the first available font sets the line metrics, and it
 * must be a text font.
 */
export function terminalFontStack(family: string): string {
  const builtIn = getComputedStyle(document.documentElement).getPropertyValue('--font-mono').trim() || 'monospace';
  const preferred = family ? [`"${family}"`] : [];
  return [...preferred, builtIn, `"${SYMBOL_FONT}"`].join(', ');
}

class PrefsState {
  value = $state<Preferences>(structuredClone(DEFAULTS));

  /** CSS font list for the terminal; see {@link terminalFontStack}. */
  get terminalFontFamily(): string {
    return terminalFontStack(this.value.terminal.font_family);
  }

  async load(): Promise<void> {
    this.value = await prefsGet();
  }

  /** Stores `next`; the backend returns the normalized value, which becomes current. */
  async save(next: Preferences): Promise<void> {
    this.value = await prefsSet(next);
  }
}

export const prefs = new PrefsState();
