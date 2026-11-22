// Picks the terminal colour scheme for the interface brightness from the preferences.
import type { ITheme } from '@xterm/xterm';
import { prefs } from '../settings/prefs.svelte';
import { schemeById, type Scheme } from './schemes';

/** Whether the interface is dark: forced by preferences / `<html data-theme>` or following the system. */
export function isDarkInterface(systemDark?: boolean): boolean {
  const mode = prefs.value.appearance.theme;
  if (mode === 'dark') return true;
  if (mode === 'light') return false;
  if (systemDark !== undefined) return systemDark;
  const forced = document.documentElement.dataset.theme;
  return forced ? forced === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
}

/** The scheme chosen for the current interface brightness. */
export function currentScheme(systemDark?: boolean): Scheme {
  const dark = isDarkInterface(systemDark);
  const { scheme_light: light, scheme_dark: darkId } = prefs.value.terminal;
  return schemeById(dark ? darkId : light, dark);
}

/** The xterm theme of {@link currentScheme}; also points `--term-bg` / `--term-fg` at its
 *  colours so the area around the terminal matches it. */
export function currentTheme(): ITheme {
  const { theme } = currentScheme();
  const root = document.documentElement.style;
  if (theme.background) root.setProperty('--term-bg', theme.background);
  if (theme.foreground) root.setProperty('--term-fg', theme.foreground);
  return theme;
}
