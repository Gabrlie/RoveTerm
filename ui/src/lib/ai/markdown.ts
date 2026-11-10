// A small Markdown parser for agent replies. Replies are untrusted, so they are never turned into
// HTML: this produces a plain structure that components render as text nodes. Supported: headings,
// paragraphs, fenced code, bullet and numbered lists, quotes, rules; inline code, bold, italics and
// links (http, https and mailto only).

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong'; children: Inline[] }
  | { kind: 'em'; children: Inline[] }
  | { kind: 'link'; href: string; children: Inline[] };

export type Block =
  | { kind: 'heading'; level: number; content: Inline[] }
  | { kind: 'paragraph'; content: Inline[] }
  | { kind: 'code'; lang: string; text: string }
  | { kind: 'list'; ordered: boolean; items: Inline[][] }
  | { kind: 'quote'; content: Inline[] }
  | { kind: 'rule' };

const SAFE_LINK = /^(https?:\/\/|mailto:)/i;

/** Parses inline markup of one block's text. */
export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  let plain = '';
  const flush = (): void => {
    if (plain) out.push({ kind: 'text', text: plain });
    plain = '';
  };
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    if (rest.startsWith('`')) {
      const end = text.indexOf('`', i + 1);
      if (end > i) {
        flush();
        out.push({ kind: 'code', text: text.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    const strong = rest.startsWith('**') ? '**' : rest.startsWith('__') ? '__' : null;
    if (strong) {
      const end = text.indexOf(strong, i + 2);
      if (end > i + 2) {
        flush();
        out.push({ kind: 'strong', children: parseInline(text.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }
    if ((rest.startsWith('*') || rest.startsWith('_')) && rest.length > 1 && rest[1] !== ' ') {
      const mark = rest[0] as string;
      const end = text.indexOf(mark, i + 1);
      // `snake_case_words` are not emphasis.
      const wordInside = mark === '_' && /\w/.test(text[i - 1] ?? '');
      if (end > i + 1 && !wordInside) {
        flush();
        out.push({ kind: 'em', children: parseInline(text.slice(i + 1, end)) });
        i = end + 1;
        continue;
      }
    }
    if (rest.startsWith('[')) {
      const match = /^\[([^\]]+)\]\(([^)\s]+)\)/.exec(rest);
      if (match) {
        flush();
        const [whole, label = '', href = ''] = match;
        if (SAFE_LINK.test(href)) out.push({ kind: 'link', href, children: parseInline(label) });
        else out.push({ kind: 'text', text: label });
        i += whole.length;
        continue;
      }
    }
    plain += text[i];
    i += 1;
  }
  flush();
  return out;
}

/** Parses a reply into blocks. */
export function parseMarkdown(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, '\n').split('\n');
  const blocks: Block[] = [];
  let paragraph: string[] = [];
  const flushParagraph = (): void => {
    if (paragraph.length) blocks.push({ kind: 'paragraph', content: parseInline(paragraph.join(' ')) });
    paragraph = [];
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i] ?? '';
    const fence = /^\s*(```|~~~)\s*([\w+-]*)/.exec(line);
    if (fence) {
      flushParagraph();
      const marker = fence[1] ?? '```';
      const code: string[] = [];
      i += 1;
      while (i < lines.length && !(lines[i] ?? '').trim().startsWith(marker)) {
        code.push(lines[i] ?? '');
        i += 1;
      }
      // An unclosed fence (still streaming) runs to the end.
      blocks.push({ kind: 'code', lang: fence[2] ?? '', text: code.join('\n') });
      i += 1;
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flushParagraph();
      blocks.push({ kind: 'heading', level: (heading[1] ?? '#').length, content: parseInline((heading[2] ?? '').trim()) });
      i += 1;
      continue;
    }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flushParagraph();
      blocks.push({ kind: 'rule' });
      i += 1;
      continue;
    }
    const item = /^\s*([-*+]|\d+[.)])\s+(.*)$/.exec(line);
    if (item) {
      flushParagraph();
      const ordered = /\d/.test(item[1] ?? '');
      const items: Inline[][] = [];
      while (i < lines.length) {
        const next = /^\s*([-*+]|\d+[.)])\s+(.*)$/.exec(lines[i] ?? '');
        if (!next || /\d/.test(next[1] ?? '') !== ordered) break;
        items.push(parseInline(next[2] ?? ''));
        i += 1;
      }
      blocks.push({ kind: 'list', ordered, items });
      continue;
    }
    if (line.startsWith('>')) {
      flushParagraph();
      const quoted: string[] = [];
      while (i < lines.length && (lines[i] ?? '').startsWith('>')) {
        quoted.push((lines[i] ?? '').replace(/^>\s?/, ''));
        i += 1;
      }
      blocks.push({ kind: 'quote', content: parseInline(quoted.join(' ')) });
      continue;
    }
    if (line.trim() === '') flushParagraph();
    else paragraph.push(line.trim());
    i += 1;
  }
  flushParagraph();
  return blocks;
}
