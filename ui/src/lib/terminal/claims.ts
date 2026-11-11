// When the copy and paste shortcuts belong to the focused terminal. Anything else keeps the
// system's own behaviour: text selected elsewhere on the page is copied as usual, and a panel
// with the keyboard focus gets the paste.

export interface ClipboardContext {
  /** Text selected in the focused terminal. */
  terminalSelection: string;
  /** Text selected in the page itself (an AI reply, a dialog). */
  pageSelection: string;
  /** The keyboard focus is in a terminal, or nowhere in particular (the page body). */
  focusInTerminal: boolean;
  /** The keyboard focus is in a text field, which handles copy and paste itself. */
  focusInTextField: boolean;
}

/** Copy takes the terminal's selection unless the page has one of its own. */
export function terminalCopies(context: ClipboardContext): boolean {
  return !context.focusInTextField && context.terminalSelection.length > 0 && context.pageSelection.trim().length === 0;
}

/** Paste goes to the terminal when it has the focus. */
export function terminalPastes(context: ClipboardContext): boolean {
  return !context.focusInTextField && context.focusInTerminal;
}
