// User preferences shared by the UI. Loaded once at startup; views read them reactively.
import { applyAppearance } from './appearance';
import { devicePrefsGet, devicePrefsSet, prefsGet, prefsSet, type DevicePrefs, type Preferences } from '../ipc';

/** Family of the bundled symbol font (see styles/fonts.css). */
export const SYMBOL_FONT = 'Symbols Nerd Font Mono';

/** Mirrors the defaults in rove-vault; used until the stored values load. */
const DEFAULTS: Preferences = {
  appearance: { theme: 'system', accent: 'blue', ui_scale: 100, language: 'auto' },
  terminal: {
    font_family: '',
    font_size: 13,
    line_height: 1.15,
    scheme_light: 'rove-light',
    scheme_dark: 'rove-dark',
    cursor_style: 'block',
    cursor_blink: false,
    scrollback: 10_000,
    copy_on_select: false,
    right_click: 'menu',
    bell: 'visual',
  },
  files: { default_conflict: 'ask', show_hidden: false, confirm_delete: true },
  status: { interval_secs: 3 },
  ai: { mode: 'ask_by_risk', providers: [], default_agent: null, read_only: [], dangerous: [], max_steps: 40, request_timeout_secs: 180, reviewer: 'model', review_model: null, review_confidence: 80, schedules: [] },
};

const DEVICE_DEFAULTS: DevicePrefs = {
  window_material: true,
  download_dir: '',
  max_transfers: 3,
  agents: { disabled: [], custom: [], auth: 'auto', expose: false },
  connection: { connect_timeout_secs: 15, keepalive_secs: 30, keepalive_max: 3 },
  external_editor: { program: '', args: [] },
  tailscale: { configured: false, auto_connect: true, control_url: '', hostname: 'RoveTerm' },
};

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
  device = $state<DevicePrefs>(structuredClone(DEVICE_DEFAULTS));

  /** CSS font list for the terminal; see {@link terminalFontStack}. */
  get terminalFontFamily(): string {
    return terminalFontStack(this.value.terminal.font_family);
  }

  async load(): Promise<void> {
    [this.value, this.device] = await Promise.all([prefsGet(), devicePrefsGet()]);
    applyAppearance(this.value.appearance);
  }

  /** The stored preferences, read again (they also change by sync); they become current. An
   *  edit starts from these, never from what the view holds: after a failed load that is the
   *  defaults, and saving them would erase the stored providers and settings. */
  async current(): Promise<Preferences> {
    this.value = await prefsGet();
    return structuredClone($state.snapshot(this.value)) as Preferences;
  }

  /** This device's stored preferences, read again; see {@link current}. */
  async currentDevice(): Promise<DevicePrefs> {
    this.device = await devicePrefsGet();
    return structuredClone($state.snapshot(this.device)) as DevicePrefs;
  }

  /** Stores `next`; the backend returns the normalized value, which becomes current. */
  async save(next: Preferences): Promise<void> {
    this.value = await prefsSet(next);
    applyAppearance(this.value.appearance);
  }

  /** Stores this device's preferences; the normalized value becomes current. */
  async saveDevice(next: DevicePrefs): Promise<void> {
    this.device = await devicePrefsSet(next);
  }
}

export const prefs = new PrefsState();
