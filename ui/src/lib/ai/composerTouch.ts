export interface ComposerKeyEvent {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  isComposing: boolean;
}

export function shouldSendOnEnter(event: ComposerKeyEvent, coarse: boolean): boolean {
  if (event.isComposing || event.key !== 'Enter') return false;
  return coarse ? event.ctrlKey || event.metaKey : !event.shiftKey;
}

export function enterKeyHint(coarse: boolean): 'enter' | 'send' {
  return coarse ? 'enter' : 'send';
}
