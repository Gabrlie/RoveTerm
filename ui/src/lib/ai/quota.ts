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

export function usageTone(percent: number | null): 'normal' | 'warning' | 'danger' {
  if (percent !== null && percent >= 95) return 'danger';
  if (percent !== null && percent >= 80) return 'warning';
  return 'normal';
}

/** Use reported context first, then estimate from the last request and configured window. */
export function contextMeter(
  usage: { used: number; size: number } | null,
  windowSize: number | null,
  stats: Parameters<typeof contextTokens>[0],
) {
  const filled = usage?.used ?? contextTokens(stats);
  const context = usage && usage.size > 0 ? usage
    : windowSize !== null && windowSize > 0 ? { used: filled ?? 0, size: windowSize }
    : null;
  const percent = contextPercent(context);
  return { context, percent, filled: percent === null ? filled : null, tone: usageTone(percent) };
}

/** `123456` as `123k`. */
export function tokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  if (count >= 1000) return `${Math.round(count / 1000)}k`;
  return String(count);
}

/** Output speed as `42 tok/s` (one decimal below 10). */
export function speed(tokensPerSecond: number): string {
  return `${tokensPerSecond < 10 ? tokensPerSecond.toFixed(1) : Math.round(tokensPerSecond)} tok/s`;
}

/** A wait as `850 ms` or `1.2 s`. */
export function wait(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

/** Tokens the conversation fills after the last request (its input and its reply), if known. */
export function contextTokens(stats: { input_tokens: number | null; output_tokens: number | null } | null): number | null {
  if (stats?.input_tokens == null) return null;
  return stats.input_tokens + (stats.output_tokens ?? 0);
}

/** Input and cached tokens summed over the requests that reported their cache. */
export interface CacheTotals {
  input: number;
  cached: number;
}

/** `totals` with request `stats` added, when it reported its cache. */
export function addCache(totals: CacheTotals, stats: { input_tokens: number | null; cached_tokens: number | null }): CacheTotals {
  if (stats.input_tokens == null || stats.cached_tokens == null) return totals;
  return { input: totals.input + stats.input_tokens, cached: totals.cached + stats.cached_tokens };
}

/** Percent of `input` tokens read from the cache, or null when nothing was counted. */
export function cacheRate(cached: number | null, input: number | null): number | null {
  if (cached == null || input == null || input <= 0) return null;
  return Math.min(100, (cached / input) * 100);
}
