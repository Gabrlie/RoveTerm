// Terminal colour schemes for the light and dark UI themes.
import type { ITheme } from '@xterm/xterm';

export const lightTheme: ITheme = {
  background: '#fbfbfc',
  foreground: '#1d2129',
  cursor: '#3b82f6',
  cursorAccent: '#fbfbfc',
  selectionBackground: '#3b82f633',
  black: '#1d2129',
  red: '#c9302c',
  green: '#2e8540',
  yellow: '#a36a00',
  blue: '#2563eb',
  magenta: '#9333ea',
  cyan: '#0e7490',
  white: '#d4d7dd',
  brightBlack: '#6b7280',
  brightRed: '#e5484d',
  brightGreen: '#3aa55a',
  brightYellow: '#c98a00',
  brightBlue: '#3b82f6',
  brightMagenta: '#a855f7',
  brightCyan: '#0891b2',
  brightWhite: '#f5f6f8',
};

export const darkTheme: ITheme = {
  background: '#121316',
  foreground: '#e6e8ec',
  cursor: '#60a5fa',
  cursorAccent: '#121316',
  selectionBackground: '#60a5fa44',
  black: '#1f2126',
  red: '#f87171',
  green: '#4ade80',
  yellow: '#facc15',
  blue: '#60a5fa',
  magenta: '#c084fc',
  cyan: '#22d3ee',
  white: '#d1d5db',
  brightBlack: '#6b7280',
  brightRed: '#fca5a5',
  brightGreen: '#86efac',
  brightYellow: '#fde047',
  brightBlue: '#93c5fd',
  brightMagenta: '#d8b4fe',
  brightCyan: '#67e8f9',
  brightWhite: '#f9fafb',
};

/** Returns the scheme matching the document's effective colour scheme. */
export function currentTheme(): ITheme {
  const forced = document.documentElement.dataset.theme;
  const dark = forced ? forced === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
  return dark ? darkTheme : lightTheme;
}
