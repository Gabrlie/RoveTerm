export type CloseChoice = 'save' | 'discard' | 'cancel';
export interface UnsavedDraft { id: string; text: string; original: string; busy: boolean }

/** Only an explicit discard or successful saving of every dirty draft permits a close. */
export async function protectDrafts<T extends UnsavedDraft>(drafts: readonly T[], choose: (dirty: T[]) => Promise<CloseChoice>, save: (draft: T) => Promise<boolean>, failed: (draft: T) => void): Promise<boolean> {
  if (drafts.some((draft) => draft.busy)) return false;
  const dirty = drafts.filter((draft) => draft.text !== draft.original);
  if (!dirty.length) return true;
  const choice = await choose(dirty);
  if (choice === 'cancel') return false;
  if (choice === 'discard') return true;
  for (const draft of dirty) {
    if (!await save(draft)) { failed(draft); return false; }
  }
  // A second view or asynchronous edit may have changed an earlier file during a batch save.
  return drafts.every((draft) => !draft.busy && draft.text === draft.original);
}
