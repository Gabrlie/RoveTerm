// External editor tasks outlive any FileBrowser. Events and the initial snapshot share revision
// ordering; file contents never pass through notifications or console logging.
import { externalEditDiscard, externalEditFinish, externalEditList, externalEditOpenCopy, externalEditRemoteVersion,
  externalEditRetry, externalEditStart, onExternalEditUpdate, type ExternalEditUpdate, type FsId } from '../ipc';
import { errorText, t } from '../i18n';
import { ask } from '../ui/confirm.svelte';
import { notify } from '../ui/toast.svelte';
import { acceptExternalUpdate } from './externalstate';

class ExternalEdits {
  byId = $state<Record<string, ExternalEditUpdate>>({});
  dialog = $state(false);
  private revisions: Record<string, number> = {};
  private sessions = new Map<string, Set<FsId>>();
  private started = false;
  private stop: (() => void) | undefined;
  private changed = new Set<(update: ExternalEditUpdate) => void>();
  get all(): ExternalEditUpdate[] { return Object.values(this.byId).sort((a, b) => a.id.localeCompare(b.id)); }
  apply(update: ExternalEditUpdate, announce = true): void {
    if (!acceptExternalUpdate(this.byId, this.revisions, update)) return;
    if (announce && ['failed', 'conflict', 'recovered'].includes(update.state)) {
      notify(t('external.attention', { path: update.path }) + (update.error ? ` ${errorText(update.error)}` : ''), 'error');
    }
    if (announce) for (const listener of this.changed) listener(update);
  }
  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    try {
      this.stop = await onExternalEditUpdate((update) => this.apply(update));
      for (const update of await externalEditList()) this.apply(update, false);
    } catch (error) {
      this.started = false;
      this.stop?.();
      this.stop = undefined;
      notify(errorText(error), 'error');
    }
  }
  listen(listener: (update: ExternalEditUpdate) => void): () => void {
    this.changed.add(listener);
    return () => this.changed.delete(listener);
  }
  dispose(): void { this.stop?.(); this.stop = undefined; this.started = false; }
  bind(label: string, id: FsId): () => void {
    const sessions = this.sessions.get(label) ?? new Set<FsId>();
    sessions.add(id);
    this.sessions.set(label, sessions);
    return () => { sessions.delete(id); if (!sessions.size) this.sessions.delete(label); };
  }
  async open(id: FsId, path: string): Promise<void> {
    try { this.apply(await externalEditStart(id, path)); this.dialog = true; }
    catch (error) { notify(errorText(error), 'error'); }
  }
  private sessionFor(row: ExternalEditUpdate): FsId | null {
    return [...(this.sessions.get(row.label) ?? [])].at(-1) ?? null;
  }
  async reopen(row: ExternalEditUpdate): Promise<void> {
    try { await externalEditOpenCopy(row.id); }
    catch (error) { notify(errorText(error), 'error'); }
  }
  async retry(row: ExternalEditUpdate, overwrite = false): Promise<void> {
    try {
      const id = this.sessionFor(row);
      const version = overwrite ? await externalEditRemoteVersion(row.id, id) : null;
      if (version && !await ask({ title: t('external.overwrite'), message: t('external.overwriteConfirm', { path: row.path, version: version.slice(0, 16) }), confirmLabel: t('external.overwrite'), danger: true })) return;
      await externalEditRetry(row.id, id, version);
    } catch (error) { notify(errorText(error), 'error'); }
  }
  async finish(row: ExternalEditUpdate): Promise<void> {
    if (!await ask({ title: t('external.finish'), message: t('external.finishConfirm', { path: row.path }), confirmLabel: t('external.finish') })) return;
    try { await externalEditFinish(row.id, this.sessionFor(row)); }
    catch (error) { notify(errorText(error), 'error'); }
  }
  async discard(row: ExternalEditUpdate): Promise<void> {
    if (!await ask({ title: t('external.discard'), message: t('external.discardConfirm', { path: row.path }), confirmLabel: t('external.discard'), danger: true })) return;
    try { await externalEditDiscard(row.id); }
    catch (error) { notify(errorText(error), 'error'); }
  }
}
export const externalEdits = new ExternalEdits();
