export interface PromptRequest {
  id: string;
  text: string;
  allowChanges: boolean;
}

/** A retry retains the original request's identity, text and authorization. */
export function requestForSend(previous: PromptRequest | null, text: string, again: boolean, allowChanges: boolean, newId: () => string): PromptRequest {
  return again && previous ? previous : { id: newId(), text, allowChanges };
}

/** An interrupted planner starts only after the user explicitly confirms recovery. */
export function canSendPrompt(busy: boolean, starting: boolean, recoveryRequired: boolean, recoveryConfirmed: boolean): boolean {
  return !busy && !starting && (!recoveryRequired || recoveryConfirmed);
}
