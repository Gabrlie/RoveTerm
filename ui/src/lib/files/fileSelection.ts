import type { DirEntry } from '../ipc';
import { formatMode, formatSize, formatTime, isEditableTextFile } from './format.ts';

export const LONG_PRESS_MS = 450;
export const LONG_PRESS_MOVE_PX = 10;

export type TouchRowAction = 'open' | 'edit' | 'menu';

export function touchRowAction(entry: Pick<DirEntry, 'kind' | 'name' | 'size'>): TouchRowAction {
  if (entry.kind === 'dir' || entry.kind === 'symlink') return 'open';
  if (entry.kind === 'file' && isEditableTextFile({ ...entry, modified: null, permissions: null })) return 'edit';
  return 'menu';
}

export function toggleSelection(selected: ReadonlySet<string>, name: string): Set<string> {
  const next = new Set(selected);
  if (next.has(name)) next.delete(name);
  else next.add(name);
  return next;
}

export function allSelected(shownNames: readonly string[], selected: ReadonlySet<string>): boolean {
  return shownNames.length > 0 && shownNames.every((name) => selected.has(name));
}

export function toggleSelectAll(shownNames: readonly string[], selected: ReadonlySet<string>): Set<string> {
  return allSelected(shownNames, selected) ? new Set() : new Set(shownNames);
}

export function exceededMoveThreshold(
  startX: number, startY: number, currentX: number, currentY: number, threshold = LONG_PRESS_MOVE_PX,
): boolean {
  return Math.hypot(currentX - startX, currentY - startY) > threshold;
}

export function formatCompactMeta(entry: Pick<DirEntry, 'kind' | 'size' | 'modified' | 'permissions'>): string {
  return [
    entry.kind === 'dir' ? '' : formatSize(entry.size),
    formatTime(entry.modified),
    formatMode(entry.permissions),
  ].filter(Boolean).join(' · ');
}
