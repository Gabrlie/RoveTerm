// How plan usage reads: which window a limit is, and how full the context is.
import type { QuotaWindow } from '../ipc';

/** A window's name: the agent's own, else by its length (`five_hours`, `week`, or minutes). */
export function windowName(window: QuotaWindow): { key: 'fiveHours' | 'week' | 'hours' | 'days' | null; count: number; label: string } {
  if (window.label) return { key: null, count: 0, label: window.label };
  const minutes = window.minutes ?? 0;
  if (minutes >= 280 && minutes <= 320) return { key: 'fiveHours', count: 5, label: '' };
  if (minutes >= 10_000 && minutes <= 10_200) return { key: 'week', count: 7, label: '' };
  if (minutes >= 1440) return { key: 'days', count: Math.round(minutes / 1440), label: '' };
  return { key: 'hours', count: Math.max(1, Math.round(minutes / 60)), label: '' };
}

/** Percent of the context window used, 0–100. */
export function contextPercent(usage: { used: number; size: number } | null): number | null {
  if (!usage || usage.size <= 0) return null;
  return Math.min(100, Math.max(0, (usage.used / usage.size) * 100));
}

/** `123456` as `123k`. */
export function tokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  if (count >= 1000) return `${Math.round(count / 1000)}k`;
  return String(count);
}
