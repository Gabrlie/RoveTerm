export interface EditorSymbol {
  label: string;
  text: string;
}

export const EDITOR_SYMBOLS: readonly EditorSymbol[] = [
  { label: 'Tab', text: '\t' },
  ...['{', '}', '[', ']', '(', ')', '"', "'", ':', ';', '=', '/', '-', '_', '|'].map((symbol) => ({
    label: symbol,
    text: symbol,
  })),
];

export function defaultWordWrap(compact: boolean): boolean {
  return compact;
}

export function shouldAutoFocusEditor(coarse: boolean): boolean {
  return !coarse;
}

export function draftViewMode(compact: boolean, selectedId: string | null): 'both' | 'list' | 'editor' {
  if (!compact) return 'both';
  return selectedId === null ? 'list' : 'editor';
}
