// Applies the appearance preferences to the document: theme, accent, interface scale, language.
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { AppearancePrefs } from '../ipc';
import { setLocale } from '../i18n';

export function applyAppearance(appearance: AppearancePrefs): void {
  const root = document.documentElement;
  if (appearance.theme === 'system') delete root.dataset.theme;
  else root.dataset.theme = appearance.theme;
  // The native window follows too: Mica and the macOS materials take the window's appearance,
  // not the page's, and would stay light behind a dark interface.
  getCurrentWindow()
    .setTheme(appearance.theme === 'system' ? null : appearance.theme)
    .catch((error: unknown) => console.warn('window theme unavailable', error));
  // Blue is the token default; other accents point at the system palette, which itself follows
  // light and dark.
  if (appearance.accent === 'blue') root.style.removeProperty('--accent');
  else root.style.setProperty('--accent', `var(--sys-${appearance.accent})`);
  root.style.setProperty('--ui-scale', String(appearance.ui_scale / 100));
  setLocale(appearance.language);
}
