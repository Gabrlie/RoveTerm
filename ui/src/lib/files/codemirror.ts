// Loaded only when a text editor is mounted. Colours follow the application's theme tokens.
import { Compartment, EditorState, type Extension } from '@codemirror/state';
import { EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view';
import { defaultKeymap, history, indentWithTab, redo, undo } from '@codemirror/commands';
import { closeSearchPanel, openSearchPanel, search, highlightSelectionMatches } from '@codemirror/search';
import { HighlightStyle, StreamLanguage, bracketMatching, indentOnInput, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { applyTextChanges, editorText, lineEnding, type TextChange } from './textformat';

async function language(path: string): Promise<Extension> {
  const ext = path.toLowerCase().split('.').at(-1);
  if (['js', 'mjs', 'cjs', 'jsx', 'ts', 'tsx'].includes(ext ?? '')) return (await import('@codemirror/lang-javascript')).javascript({ typescript: ext === 'ts' || ext === 'tsx', jsx: ext === 'jsx' || ext === 'tsx' });
  if (ext === 'json') return (await import('@codemirror/lang-json')).json();
  if (ext === 'py') return (await import('@codemirror/lang-python')).python();
  if (['html', 'htm', 'svelte', 'vue'].includes(ext ?? '')) return (await import('@codemirror/lang-html')).html();
  if (['css', 'scss', 'less'].includes(ext ?? '')) return (await import('@codemirror/lang-css')).css();
  if (['xml', 'svg'].includes(ext ?? '')) return (await import('@codemirror/lang-xml')).xml();
  if (['md', 'markdown'].includes(ext ?? '')) return (await import('@codemirror/lang-markdown')).markdown();
  if (ext === 'sql') return (await import('@codemirror/lang-sql')).sql();
  if (['yaml', 'yml'].includes(ext ?? '')) return StreamLanguage.define((await import('@codemirror/legacy-modes/mode/yaml')).yaml);
  if (ext === 'toml') return StreamLanguage.define((await import('@codemirror/legacy-modes/mode/toml')).toml);
  if (['sh', 'bash', 'zsh'].includes(ext ?? '') || /\/(\.?bashrc|\.?zshrc|profile)$/.test(path)) return StreamLanguage.define((await import('@codemirror/legacy-modes/mode/shell')).shell);
  if (ext === 'rs') return StreamLanguage.define((await import('@codemirror/legacy-modes/mode/rust')).rust);
  if (ext === 'go') return StreamLanguage.define((await import('@codemirror/legacy-modes/mode/go')).go);
  if (['c', 'h', 'cpp', 'hpp', 'java', 'cs'].includes(ext ?? '')) {
    const modes = await import('@codemirror/legacy-modes/mode/clike');
    return StreamLanguage.define(ext === 'java' ? modes.java : ext === 'cs' ? modes.csharp : ext === 'cpp' || ext === 'hpp' ? modes.cpp : modes.c);
  }
  if (/\/(dockerfile|containerfile)(\.[^/]*)?$/i.test(path)) return StreamLanguage.define((await import('@codemirror/legacy-modes/mode/dockerfile')).dockerFile);
  if (path.includes('/nginx/') || /\/nginx\.conf$/i.test(path)) return StreamLanguage.define((await import('@codemirror/legacy-modes/mode/nginx')).nginx);
  if (['ini', 'conf', 'cfg', 'env'].includes(ext ?? '')) return StreamLanguage.define((await import('@codemirror/legacy-modes/mode/properties')).properties);
  return [];
}

const theme = EditorView.theme({
  '&': { height: '100%', color: 'var(--fg)', backgroundColor: 'var(--surface)', fontSize: 'var(--text-body)' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { overflow: 'auto', WebkitOverflowScrolling: 'touch', fontFamily: 'var(--font-mono)', lineHeight: '1.6' },
  '.cm-content': { caretColor: 'var(--accent)', padding: 'var(--space-2) 0' },
  '.cm-line': { padding: '0 var(--space-3)' },
  '.cm-gutters': { backgroundColor: 'var(--editor-gutter)', color: 'var(--fg-subtle)', borderRight: '1px solid var(--separator)' },
  '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'var(--editor-active-line)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: 'var(--editor-selection)' },
  '.cm-cursor': { borderLeftColor: 'var(--accent)' },
  '.cm-searchMatch': { backgroundColor: 'var(--warning-soft)', outline: '1px solid var(--warning)' },
  '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'var(--accent-soft)', outlineColor: 'var(--accent)' },
  '.cm-panels': { backgroundColor: 'var(--material-bar)', color: 'var(--fg)', borderColor: 'var(--separator)' },
  '.cm-panel.cm-search': {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: 'var(--space-1)',
    padding: 'var(--space-2)',
    fontFamily: 'var(--font-ui)',
    fontSize: 'var(--text-footnote)',
    '& > input, & > button, & > label': { margin: '0' },
    '& > label': { display: 'inline-flex', flexDirection: 'row', alignItems: 'center', whiteSpace: 'normal' },
    '& > br': { display: 'none' },
    '& [name=close]': { position: 'static', minWidth: 'var(--control-height)' },
    '& .cm-textfield, & .cm-button': { minWidth: '0', maxWidth: '100%' },
  },
  '@media (pointer: coarse)': {
    '.cm-panel.cm-search .cm-textfield, .cm-panel.cm-search button': { minHeight: 'var(--control-height)' },
  },
  '.cm-textfield': { background: 'var(--surface)', color: 'var(--fg)', border: '1px solid var(--border)', borderRadius: 'var(--radius-xs)' },
  '.cm-button': { background: 'var(--fill)', color: 'var(--fg)', border: '1px solid var(--border)', borderRadius: 'var(--radius-xs)' },
});
const highlight = syntaxHighlighting(HighlightStyle.define([
  { tag: [tags.keyword, tags.modifier, tags.operatorKeyword], color: 'var(--editor-keyword)' },
  { tag: [tags.string, tags.regexp], color: 'var(--editor-string)' },
  { tag: [tags.number, tags.bool, tags.null], color: 'var(--editor-number)' },
  { tag: [tags.typeName, tags.className, tags.tagName], color: 'var(--editor-type)' },
  { tag: [tags.comment, tags.meta], color: 'var(--editor-comment)' },
  { tag: [tags.heading, tags.strong], fontWeight: 'bold' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.link, textDecoration: 'underline' },
]));

export async function createEditor(
  parent: HTMLElement,
  initial: string,
  path: string,
  phrases: Record<string, string>,
  onchange: (text: string) => void,
  options?: { wordWrap?: boolean },
) {
  let raw = initial;
  let preferredEnding = lineEnding(initial);
  let replacing = false;
  const editable = new Compartment();
  const lang = new Compartment();
  const wordWrap = new Compartment();
  const view = new EditorView({ parent, state: EditorState.create({ doc: editorText(raw), extensions: [
    lineNumbers(), history(), drawSelection(), highlightActiveLine(), highlightActiveLineGutter(),
    indentOnInput(), bracketMatching(), search(), highlightSelectionMatches(), theme, highlight,
    EditorState.phrases.of(phrases), editable.of([EditorView.editable.of(true), EditorState.readOnly.of(false)]), lang.of([]),
    wordWrap.of(options?.wordWrap ? EditorView.lineWrapping : []),
    keymap.of([...defaultKeymap, indentWithTab, { key: 'Escape', run: closeSearchPanel }]),
    EditorView.updateListener.of((update) => {
      if (!update.docChanged || replacing) return;
      const changes: TextChange[] = [];
      update.changes.iterChanges((from, to, _fromB, _toB, insert) => changes.push({ from, to, insert: insert.toString() }));
      raw = applyTextChanges(raw, changes, preferredEnding);
      onchange(raw);
    }),
    EditorView.contentAttributes.of({ 'aria-label': path, spellcheck: 'false' }),
  ] }) });
  let destroyed = false;
  // A missing highlighter still leaves the file editable as plain text.
  void language(path).then((extension) => { if (!destroyed) view.dispatch({ effects: lang.reconfigure(extension) }); }).catch(() => {});
  return {
    focus: () => view.focus(),
    find: () => { openSearchPanel(view); },
    undo: () => { undo(view); },
    redo: () => { redo(view); },
    setEditable: (value: boolean) => view.dispatch({ effects: editable.reconfigure([EditorView.editable.of(value), EditorState.readOnly.of(!value)]) }),
    setWordWrap: (enabled: boolean) => view.dispatch({ effects: wordWrap.reconfigure(enabled ? EditorView.lineWrapping : []) }),
    insertText: (text: string) => {
      if (view.state.readOnly || !view.state.facet(EditorView.editable)) return;
      view.dispatch(view.state.replaceSelection(text), { scrollIntoView: true, userEvent: 'input.type' });
    },
    sync: (text: string) => {
      if (text !== raw) preferredEnding = lineEnding(text);
      raw = text;
      const next = editorText(text);
      if (next === view.state.doc.toString()) return;
      replacing = true;
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
      replacing = false;
    },
    destroy: () => { destroyed = true; view.destroy(); },
  };
}
export type EditorController = Awaited<ReturnType<typeof createEditor>>;
