// How transfers read in the downloads popover and the download center.
import type { TransferUpdate } from '../ipc';

export function isFinal(row: TransferUpdate): boolean {
  return row.state === 'done' || row.state === 'failed' || row.state === 'cancelled';
}

/** Percent copied, 0–100. */
export function progress(row: TransferUpdate): number {
  if (row.bytes_total > 0) return Math.min(100, (row.bytes_done / row.bytes_total) * 100);
  return row.state === 'done' ? 100 : 0;
}

/** Newest first, at most `limit`. */
export function newestFirst(rows: readonly TransferUpdate[], limit = Infinity): TransferUpdate[] {
  return [...rows].sort((a, b) => b.id.localeCompare(a.id)).slice(0, limit);
}

/** Transfers still going and how far they are together (`null` without any). */
export function activity(rows: readonly TransferUpdate[]): { active: number; percent: number | null } {
  const going = rows.filter((row) => !isFinal(row));
  const total = going.reduce((sum, row) => sum + row.bytes_total, 0);
  const done = going.reduce((sum, row) => sum + Math.min(row.bytes_done, row.bytes_total), 0);
  return { active: going.length, percent: going.length === 0 ? null : total > 0 ? (done / total) * 100 : 0 };
}
