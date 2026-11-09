// Applies the appearance preferences to the document: theme, accent, interface scale, language.
import type { AppearancePrefs } from '../ipc';
import { setLocale } from '../i18n';

export function applyAppearance(appearance: AppearancePrefs): void {
  const root = document.documentElement;
  if (appearance.theme === 'system') delete root.dataset.theme;
  else root.dataset.theme = appearance.theme;
  // Blue is the token default; other accents point at the system palette, which itself follows
  // light and dark.
  if (appearance.accent === 'blue') root.style.removeProperty('--accent');
  else root.style.setProperty('--accent', `var(--sys-${appearance.accent})`);
  root.style.setProperty('--ui-scale', String(appearance.ui_scale / 100));
  setLocale(appearance.language);
}
