// Draft text lives only in this application instance. Each draft owns its SFTP session so
// unmounting a tab or evicting a cached browser cannot drop its connection or contents.
import { isCommandError, sftpClose, sftpOpen, sftpReadText, sftpRetain, sftpSaveText, type FsId, type RemoteSource } from '../ipc';
import { errorText, t } from '../i18n';
import { ask } from '../ui/confirm.svelte';
import { notify } from '../ui/toast.svelte';
import { protectDrafts, type CloseChoice } from './draftclose';

export interface EditDraft {
  id: string;
  key: string;
  source: RemoteSource;
  fs: FsId;
  label: string;
  path: string;
  text: string;
  original: string;
  version: string;
  busy: boolean;
  conflict: boolean;
  error: string;
}
export type EditorCommand = 'editor.save' | 'editor.find' | 'editor.undo' | 'editor.redo' | 'editor.close';
export type { CloseChoice } from './draftclose';

class Editors {
  drafts = $state<EditDraft[]>([]);
  /** The application-level drafts view; it is also available after its browser disappears. */
  showDrafts = $state(false);
  selected = $state<string | null>(null);
  exiting = $state(false);
  closingIds = $state<string[]>([]);
  private protecting = false;
  closing = $state<{ ids: string[]; resolve: (choice: CloseChoice) => void } | null>(null);
  private opening = new Map<string, Promise<EditDraft>>();
  private focusedCommand: ((command: EditorCommand) => void) | null = null;
  focused: string | null = null;

  get dirty(): EditDraft[] { return this.drafts.filter((draft) => draft.text !== draft.original); }
  get pending(): boolean { return this.drafts.some((draft) => draft.busy || draft.text !== draft.original); }
  find(id: string): EditDraft | undefined { return this.drafts.find((draft) => draft.id === id); }

  async open(fs: FsId, label: string, source: RemoteSource, path: string): Promise<EditDraft> {
    const identity = JSON.stringify([label, path]);
    const pending = this.opening.get(identity);
    if (pending) return pending;
    const promise = (async () => {
      const retained = await sftpRetain(fs);
      const existing = this.drafts.find((draft) => draft.key === identity);
      if (existing) {
        if (existing.busy) {
          await this.release(retained);
        } else {
          const previous = existing.fs;
          existing.fs = retained;
          existing.source = { ...source };
          await this.release(previous);
        }
        return existing;
      }
      try {
        const snapshot = await sftpReadText(retained, path);
        this.drafts.push({ id: retained, key: identity, source: { ...source }, fs: retained, label, path,
          text: snapshot.text, original: snapshot.text, version: snapshot.version, busy: false, conflict: false, error: '' });
        return this.find(retained)!;
      } catch (error) {
        await sftpClose(retained).catch(() => { /* The app also releases handles on shutdown. */ });
        throw error;
      }
    })();
    this.opening.set(identity, promise);
    try { return await promise; } finally { this.opening.delete(identity); }
  }

  async save(draft: EditDraft, version = draft.version): Promise<boolean> {
    if (draft.busy) return false;
    draft.busy = true;
    draft.error = '';
    const text = draft.text;
    try {
      const saved = await sftpSaveText(draft.fs, draft.path, version, text);
      draft.version = saved;
      draft.original = text;
      draft.conflict = false;
      return true;
    } catch (error) {
      draft.conflict = isCommandError(error) && error.code === 'fs.edit_conflict';
      draft.error = draft.conflict ? '' : errorText(error);
      return false;
    } finally { draft.busy = false; }
  }

  async reload(draft: EditDraft): Promise<void> {
    if (draft.busy) return;
    if (draft.text !== draft.original && !await ask({ title: t('editor.reload'), message: t('editor.reloadConfirm', { path: draft.path }), confirmLabel: t('editor.reload'), danger: true })) return;
    draft.busy = true;
    try {
      const snapshot = await sftpReadText(draft.fs, draft.path);
      draft.text = draft.original = snapshot.text;
      draft.version = snapshot.version;
      draft.conflict = false;
      draft.error = '';
    } catch (error) { draft.error = errorText(error); }
    finally { draft.busy = false; }
  }

  async overwrite(draft: EditDraft): Promise<void> {
    if (draft.busy) return;
    draft.busy = true;
    try {
      const current = await sftpReadText(draft.fs, draft.path);
      const approved = await ask({ title: t('editor.overwriteTitle'),
        message: t('editor.overwriteConfirm', { path: draft.path, old: draft.version.slice(0, 16), next: current.version.slice(0, 16) }),
        confirmLabel: t('editor.overwrite'), danger: true });
      if (!approved) return;
      // Approval applies only to this freshly read version. A second remote change still conflicts.
      draft.busy = false;
      await this.save(draft, current.version);
    } catch (error) { draft.error = errorText(error); }
    finally { draft.busy = false; }
  }

  async reconnect(draft: EditDraft): Promise<void> {
    if (draft.busy) return;
    draft.busy = true;
    try {
      const opened = await sftpOpen(draft.source, null, false);
      if (opened.label !== draft.label) {
        await this.release(opened.id);
        draft.error = t('editor.reconnectTargetChanged');
        return;
      }
      const previous = draft.fs;
      draft.fs = opened.id;
      draft.error = '';
      await this.release(previous);
    } catch (error) { draft.error = errorText(error); }
    finally { draft.busy = false; }
  }

  private async release(fs: FsId): Promise<void> {
    try { await sftpClose(fs); }
    catch { /* Application shutdown also releases any remaining file handles. */ }
  }

  async canClose(ids: string[]): Promise<boolean> {
    if (this.protecting) return false;
    const drafts = ids.map((id) => this.find(id)).filter((draft): draft is EditDraft => !!draft);
    this.protecting = true;
    this.closingIds = ids;
    try {
      return await protectDrafts(drafts, async (dirty) => {
        const choice = await new Promise<CloseChoice>((resolve) => { this.closing = { ids: dirty.map((draft) => draft.id), resolve }; });
        this.closing = null;
        return choice;
      }, (draft) => this.save(draft), (draft) => { this.selected = draft.id; this.showDrafts = true; });
    } finally { this.closing = null; this.closingIds = []; this.protecting = false; }
  }

  async close(id: string): Promise<boolean> {
    if (!await this.canClose([id])) return false;
    const draft = this.find(id);
    this.drafts = this.drafts.filter((entry) => entry.id !== id);
    if (this.selected === id) this.selected = null;
    if (this.focused === id) { this.focusedCommand = null; this.focused = null; }
    if (draft) await sftpClose(draft.fs).catch(() => {});
    return true;
  }

  detached(id: string): void {
    const draft = this.find(id);
    if (draft && draft.text !== draft.original) notify(t('editor.retained', { path: draft.path }), 'info');
  }

  focus(id: string, handler: (command: EditorCommand) => void): void {
    this.focused = id;
    this.focusedCommand = handler;
  }
  register(_id: string, handler: (command: EditorCommand) => void): () => void {
    return () => {
      if (this.focusedCommand === handler) { this.focusedCommand = null; this.focused = null; }
    };
  }
  run(command: EditorCommand): boolean {
    const handler = this.focusedCommand;
    if (!handler) return false;
    handler(command);
    return true;
  }
}

export const editors = new Editors();
