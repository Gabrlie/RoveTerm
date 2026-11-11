// Markdown of agent replies (GitHub flavoured). Replies are untrusted, so they are never turned
// into HTML: `marked` only tokenizes, and the tokens become a plain structure that components
// render as elements and text nodes. HTML in a reply stays visible text; links keep only http,
// https and mailto addresses.
import { Lexer, type Token, type Tokens } from 'marked';

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong'; children: Inline[] }
  | { kind: 'em'; children: Inline[] }
  | { kind: 'del'; children: Inline[] }
  | { kind: 'link'; href: string; children: Inline[] }
  | { kind: 'break' };

export type Align = 'left' | 'center' | 'right' | null;

export interface ListItem {
  /** `true` / `false` for a task item (checked or not); `null` for a plain one. */
  task: boolean | null;
  blocks: Block[];
}

export type Block =
  | { kind: 'heading'; level: number; content: Inline[] }
  | { kind: 'paragraph'; content: Inline[] }
  | { kind: 'code'; lang: string; text: string }
  | { kind: 'list'; ordered: boolean; start: number; items: ListItem[] }
  | { kind: 'quote'; blocks: Block[] }
  | { kind: 'table'; align: Align[]; header: Inline[][]; rows: Inline[][][] }
  | { kind: 'rule' };

const SAFE_LINK = /^(https?:\/\/|mailto:)/i;

function text(value: string): Inline[] {
  return value ? [{ kind: 'text', text: value }] : [];
}

function inlines(tokens: Token[] | undefined, fallback = ''): Inline[] {
  if (!tokens) return text(fallback);
  return tokens.flatMap((token): Inline[] => {
    switch (token.type) {
      case 'text':
        // Text inside a tight list item may carry its own inline tokens.
        return 'tokens' in token && token.tokens ? inlines(token.tokens) : text(token.text);
      case 'escape':
        return text(token.text);
      case 'codespan':
        return [{ kind: 'code', text: unescapeEntities(token.text) }];
      case 'strong':
        return [{ kind: 'strong', children: inlines(token.tokens, token.text) }];
      case 'em':
        return [{ kind: 'em', children: inlines(token.tokens, token.text) }];
      case 'del':
        return [{ kind: 'del', children: inlines(token.tokens, token.text) }];
      case 'br':
        return [{ kind: 'break' }];
      case 'link': {
        const link = token as Tokens.Link;
        const children = inlines(link.tokens, link.text);
        return SAFE_LINK.test(link.href) ? [{ kind: 'link', href: link.href, children }] : children;
      }
      case 'image': {
        // Images are not loaded; their description stays, linked when the address is safe.
        const image = token as Tokens.Image;
        const label = text(image.text || image.href);
        return SAFE_LINK.test(image.href) ? [{ kind: 'link', href: image.href, children: label }] : label;
      }
      case 'html':
        return text(token.raw);
      default:
        return text('raw' in token ? String(token.raw) : '');
    }
  });
}

/** `marked` escapes `&`, `<` and `>` in code spans for HTML output; the text is shown as text. */
function unescapeEntities(value: string): string {
  return value.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&');
}

function blocks(tokens: Token[]): Block[] {
  return tokens.flatMap((token): Block[] => {
    switch (token.type) {
      case 'heading':
        return [{ kind: 'heading', level: token.depth, content: inlines(token.tokens, token.text) }];
      case 'paragraph':
        return [{ kind: 'paragraph', content: inlines(token.tokens, token.text) }];
      case 'text':
        return [{ kind: 'paragraph', content: 'tokens' in token && token.tokens ? inlines(token.tokens) : text(token.text) }];
      case 'code':
        return [{ kind: 'code', lang: (token.lang ?? '').split(/\s/)[0] ?? '', text: token.text }];
      case 'list': {
        const list = token as Tokens.List;
        return [
          {
            kind: 'list',
            ordered: list.ordered,
            start: typeof list.start === 'number' ? list.start : 1,
            items: list.items.map((item) => ({
              task: item.task ? item.checked === true : null,
              // The checkbox is shown from `task`; marked also puts it in the item's text.
              blocks: blocks(item.tokens.filter((child) => child.type !== 'checkbox')),
            })),
          },
        ];
      }
      case 'blockquote':
        return [{ kind: 'quote', blocks: blocks(token.tokens ?? []) }];
      case 'table': {
        const table = token as Tokens.Table;
        return [
          {
            kind: 'table',
            align: table.align.map((align) => align ?? null),
            header: table.header.map((cell) => inlines(cell.tokens, cell.text)),
            rows: table.rows.map((row) => row.map((cell) => inlines(cell.tokens, cell.text))),
          },
        ];
      }
      case 'hr':
        return [{ kind: 'rule' }];
      case 'html':
        return [{ kind: 'paragraph', content: text(token.raw.trim()) }];
      case 'space':
      case 'def':
        return [];
      default:
        return 'raw' in token && String(token.raw).trim() ? [{ kind: 'paragraph', content: text(String(token.raw).trim()) }] : [];
    }
  });
}

/** Parses a reply; an unclosed code fence (a reply still streaming) runs to the end. */
export function parseMarkdown(source: string): Block[] {
  return blocks(new Lexer({ gfm: true, breaks: false }).lex(source));
}

/** Parses the inline markup of one line. */
export function parseInline(source: string): Inline[] {
  return inlines(new Lexer({ gfm: true }).inlineTokens(source));
}
