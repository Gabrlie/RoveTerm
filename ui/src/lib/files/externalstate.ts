import type { ExternalEditUpdate } from '../ipc';

/** Keep closed revisions as tombstones: a slow list response cannot resurrect a closed row. */
export function acceptExternalUpdate(rows: Record<string, ExternalEditUpdate>, revisions: Record<string, number>, update: ExternalEditUpdate): boolean {
  if (!Number.isSafeInteger(update.revision) || update.revision < 0 || update.revision <= (revisions[update.id] ?? -1)) return false;
  revisions[update.id] = update.revision;
  if (update.state === 'closed') delete rows[update.id];
  else rows[update.id] = update;
  return true;
}
