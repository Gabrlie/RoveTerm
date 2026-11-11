// OSC 52: programs in the terminal (Claude Code, tmux, vim) put their own selection on the
// clipboard with `ESC ] 52 ; <targets> ; <base64> BEL`. Writes are honoured; a query (`?`),
// which would hand the clipboard to the program, never is.

/** Largest text accepted from one sequence. */
export const MAX_OSC52 = 1024 * 1024;

/** The text an OSC 52 sequence's data (after `52;`) puts on the clipboard; `null` for a query,
 *  an empty or malformed one, or one too large. */
export function osc52Text(data: string): string | null {
  const split = data.indexOf(';');
  if (split < 0) return null;
  const targets = data.slice(0, split);
  const payload = data.slice(split + 1).trim();
  if (!/^[cpqs0-7]*$/.test(targets)) return null;
  if (!payload || payload === '?' || payload.length > (MAX_OSC52 * 4) / 3 + 4) return null;
  let binary: string;
  try {
    binary = atob(payload);
  } catch {
    return null;
  }
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  const text = new TextDecoder('utf-8', { fatal: false }).decode(bytes);
  return text.length > 0 ? text : null;
}
