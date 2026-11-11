// Reading and writing the clipboard. The native plugin is used instead of `navigator.clipboard.readText()`,
// which makes WebKit show a "Paste" confirmation bubble on macOS before every paste.
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';

/** The clipboard's text; empty when it holds none or cannot be read. */
export async function readClipboard(): Promise<string> {
  try {
    return (await readText()) ?? '';
  } catch (error) {
    console.warn('reading the clipboard failed', error);
    return '';
  }
}

/** Puts `text` on the clipboard through the native plugin (WebKit may refuse the web API when
 *  the page does not hold the focus), falling back to the web API. */
export async function writeClipboard(text: string): Promise<void> {
  try {
    await writeText(text);
  } catch (error) {
    console.warn('native clipboard write failed', error);
    await navigator.clipboard.writeText(text);
  }
}
