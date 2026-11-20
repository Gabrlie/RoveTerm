// CodeMirror uses LF internally. Apply its ranges to the original text so untouched CRLF,
// lone CR and BOM bytes survive editing, including files with mixed line endings.
export interface TextChange { from: number; to: number; insert: string }

export function editorText(text: string): string {
  return text.replace(/^\uFEFF/, '').replace(/\r\n?|\n/g, '\n');
}

export function lineEnding(text: string): '\r\n' | '\r' | '\n' {
  const endings = text.match(/\r\n|\r|\n/g) ?? [];
  const counts = new Map<string, number>();
  for (const ending of endings) counts.set(ending, (counts.get(ending) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1])[0]?.[0] as '\r\n' | '\r' | '\n' ?? '\n';
}

/** Ranges refer to the old LF document and are ordered, as ChangeSet.iterChanges provides. */
export function applyTextChanges(text: string, changes: readonly TextChange[], preferredEnding = lineEnding(text)): string {
  const bom = text.startsWith('\uFEFF') ? '\uFEFF' : '';
  const raw = text.slice(bom.length);
  const ending = preferredEnding;
  let rawAt = 0;
  let docAt = 0;
  let copied = 0;
  let result = bom;
  const offset = (position: number): number => {
    while (docAt < position && rawAt < raw.length) {
      if (raw[rawAt] === '\r' && raw[rawAt + 1] === '\n') rawAt += 2;
      else rawAt += 1;
      docAt += 1;
    }
    return rawAt;
  };
  for (const change of changes) {
    const from = offset(change.from);
    const to = offset(change.to);
    result += raw.slice(copied, from) + change.insert.replace(/\r\n?|\n/g, ending);
    copied = to;
  }
  return result + raw.slice(copied);
}
