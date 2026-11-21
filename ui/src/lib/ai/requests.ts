export interface PromptRequest {
  id: string;
  text: string;
}

/** A retry retains the original request's identity and text. */
export function requestForSend(previous: PromptRequest | null, text: string, again: boolean, newId: () => string): PromptRequest {
  return again && previous ? previous : { id: newId(), text };
}

/** An interrupted planner starts only after the user explicitly confirms recovery. */
export function canSendPrompt(busy: boolean, starting: boolean, recoveryRequired: boolean, recoveryConfirmed: boolean): boolean {
  return !busy && !starting && (!recoveryRequired || recoveryConfirmed);
}
