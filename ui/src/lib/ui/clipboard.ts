// Reading the clipboard. The native plugin is used instead of `navigator.clipboard.readText()`,
// which makes WebKit show a "Paste" confirmation bubble on macOS before every paste.
import { readText } from '@tauri-apps/plugin-clipboard-manager';

/** The clipboard's text; empty when it holds none or cannot be read. */
export async function readClipboard(): Promise<string> {
  try {
    return (await readText()) ?? '';
  } catch (error) {
    console.warn('reading the clipboard failed', error);
    return '';
  }
}
