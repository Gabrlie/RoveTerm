// Terminal colour schemes. Ids are stored in preferences (`scheme_light` / `scheme_dark`); an
// unknown id falls back to the RoveTerm scheme of the same brightness. Third-party palettes are
// MIT-licensed; sources are listed in SCHEMES.md next to this file.
import type { ITheme } from '@xterm/xterm';

export interface Scheme {
  id: string;
  /** Display name; scheme names are proper nouns and are not translated. */
  name: string;
  dark: boolean;
  theme: ITheme;
}

/** Builds a theme from the 16 ANSI colours in order (black … white, then the bright ones). */
function palette(
  base: { background: string; foreground: string; cursor: string; selection: string },
  ansi: [string, string, string, string, string, string, string, string, string, string, string, string, string, string, string, string],
): ITheme {
  const [black, red, green, yellow, blue, magenta, cyan, white, brightBlack, brightRed, brightGreen, brightYellow, brightBlue, brightMagenta, brightCyan, brightWhite] = ansi;
  return {
    background: base.background,
    foreground: base.foreground,
    cursor: base.cursor,
    cursorAccent: base.background,
    selectionBackground: base.selection,
    black,
    red,
    green,
    yellow,
    blue,
    magenta,
    cyan,
    white,
    brightBlack,
    brightRed,
    brightGreen,
    brightYellow,
    brightBlue,
    brightMagenta,
    brightCyan,
    brightWhite,
  };
}

export const SCHEMES: Scheme[] = [
  {
    id: 'rove-light',
    name: 'RoveTerm Light',
    dark: false,
    theme: palette(
      { background: '#ffffff', foreground: '#1d1d1f', cursor: '#007aff', selection: '#007aff33' },
      ['#1d1d1f', '#d70015', '#248a3d', '#b25000', '#0066cc', '#8944ab', '#0e7490', '#c7c7cc', '#6e6e73', '#ff3b30', '#34c759', '#c98a00', '#007aff', '#af52de', '#30b0c7', '#f2f2f7'],
    ),
  },
  {
    id: 'rove-dark',
    name: 'RoveTerm Dark',
    dark: true,
    theme: palette(
      { background: '#1a1a1c', foreground: '#f2f2f4', cursor: '#0a84ff', selection: '#0a84ff44' },
      ['#2c2c2e', '#ff453a', '#32d74b', '#ffd60a', '#0a84ff', '#bf5af2', '#64d2ff', '#d1d1d6', '#8e8e93', '#ff6961', '#6ae38a', '#ffe55c', '#5eb0ff', '#da8fff', '#8be3ff', '#ffffff'],
    ),
  },
  {
    id: 'github-light',
    name: 'GitHub Light',
    dark: false,
    theme: palette(
      { background: '#ffffff', foreground: '#24292f', cursor: '#0969da', selection: '#0969da33' },
      ['#24292f', '#cf222e', '#116329', '#4d2d00', '#0969da', '#8250df', '#1b7c83', '#6e7781', '#57606a', '#a40e26', '#1a7f37', '#633c01', '#218bff', '#a475f9', '#3192aa', '#8c959f'],
    ),
  },
  {
    id: 'solarized-light',
    name: 'Solarized Light',
    dark: false,
    theme: palette(
      { background: '#fdf6e3', foreground: '#657b83', cursor: '#586e75', selection: '#eee8d5' },
      ['#073642', '#dc322f', '#859900', '#b58900', '#268bd2', '#d33682', '#2aa198', '#eee8d5', '#002b36', '#cb4b16', '#586e75', '#657b83', '#839496', '#6c71c4', '#93a1a1', '#fdf6e3'],
    ),
  },
  {
    id: 'solarized-dark',
    name: 'Solarized Dark',
    dark: true,
    theme: palette(
      { background: '#002b36', foreground: '#839496', cursor: '#93a1a1', selection: '#073642' },
      ['#073642', '#dc322f', '#859900', '#b58900', '#268bd2', '#d33682', '#2aa198', '#eee8d5', '#586e75', '#cb4b16', '#859900', '#b58900', '#268bd2', '#6c71c4', '#2aa198', '#fdf6e3'],
    ),
  },
  {
    id: 'one-dark',
    name: 'One Dark',
    dark: true,
    theme: palette(
      { background: '#282c34', foreground: '#abb2bf', cursor: '#528bff', selection: '#3e4451' },
      ['#3f4451', '#e05561', '#8cc265', '#d18f52', '#4aa5f0', '#c162de', '#42b3c2', '#e6e6e6', '#4f5666', '#ff616e', '#a5e075', '#f0a45d', '#4dc4ff', '#de73ff', '#4cd1e0', '#ffffff'],
    ),
  },
  {
    id: 'dracula',
    name: 'Dracula',
    dark: true,
    theme: palette(
      { background: '#282a36', foreground: '#f8f8f2', cursor: '#f8f8f2', selection: '#44475a' },
      ['#21222c', '#ff5555', '#50fa7b', '#f1fa8c', '#bd93f9', '#ff79c6', '#8be9fd', '#f8f8f2', '#6272a4', '#ff6e6e', '#69ff94', '#ffffa5', '#d6acff', '#ff92df', '#a4ffff', '#ffffff'],
    ),
  },
  {
    id: 'nord',
    name: 'Nord',
    dark: true,
    theme: palette(
      { background: '#2e3440', foreground: '#d8dee9', cursor: '#d8dee9', selection: '#434c5e' },
      ['#3b4252', '#bf616a', '#a3be8c', '#ebcb8b', '#81a1c1', '#b48ead', '#88c0d0', '#e5e9f0', '#4c566a', '#bf616a', '#a3be8c', '#ebcb8b', '#81a1c1', '#b48ead', '#8fbcbb', '#eceff4'],
    ),
  },
  {
    id: 'gruvbox-dark',
    name: 'Gruvbox Dark',
    dark: true,
    theme: palette(
      { background: '#282828', foreground: '#ebdbb2', cursor: '#ebdbb2', selection: '#504945' },
      ['#282828', '#cc241d', '#98971a', '#d79921', '#458588', '#b16286', '#689d6a', '#a89984', '#928374', '#fb4934', '#b8bb26', '#fabd2f', '#83a598', '#d3869b', '#8ec07c', '#ebdbb2'],
    ),
  },
  {
    id: 'tokyo-night',
    name: 'Tokyo Night',
    dark: true,
    theme: palette(
      { background: '#1a1b26', foreground: '#c0caf5', cursor: '#c0caf5', selection: '#33467c' },
      ['#15161e', '#f7768e', '#9ece6a', '#e0af68', '#7aa2f7', '#bb9af7', '#7dcfff', '#a9b1d6', '#414868', '#f7768e', '#9ece6a', '#e0af68', '#7aa2f7', '#bb9af7', '#7dcfff', '#c0caf5'],
    ),
  },
];

/** The scheme with `id`, or the RoveTerm scheme for the interface brightness when unknown. */
export function schemeById(id: string, dark: boolean): Scheme {
  const fallback = dark ? 'rove-dark' : 'rove-light';
  return SCHEMES.find((scheme) => scheme.id === id) ?? SCHEMES.find((scheme) => scheme.id === fallback) ?? (SCHEMES[0] as Scheme);
}
