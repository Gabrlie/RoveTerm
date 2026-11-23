export interface PromptRequest {
  id: string;
  text: string;
}

/** A retry retains the original request's identity and text. */
export function requestForSend(previous: PromptRequest | null, text: string, again: boolean, newId: () => string): PromptRequest {
  return again && previous ? previous : { id: newId(), text };
}

export function canSendPrompt(busy: boolean, starting: boolean): boolean {
  return !busy && !starting;
}
