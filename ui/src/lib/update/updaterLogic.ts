// Pure helpers for update state presentation, channel resolution, skipped-version checks and download progress.
import type { UpdateChannel } from '../ipc';

export type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'up_to_date'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'ready'
  | 'error';

/** Strips whitespace and an optional leading `v`/`V` so versions compare consistently. */
export function normalizeVersion(version: string | null | undefined): string {
  if (!version) return '';
  return version.trim().replace(/^v/i, '');
}

/** `true` when the normalized SemVer version carries a pre-release identifier (e.g. `-beta.1`). */
export function isPrereleaseVersion(version: string | null | undefined): boolean {
  const normalized = normalizeVersion(version);
  const core = normalized.split('+', 1)[0] ?? '';
  return core.includes('-');
}

/** Resolves the active update channel from stored preference or the running binary's version. */
export function effectiveUpdateChannel(
  stored: UpdateChannel | null | undefined,
  currentVersion: string | null | undefined,
): UpdateChannel {
  if (stored === 'stable' || stored === 'beta') return stored;
  return isPrereleaseVersion(currentVersion) ? 'beta' : 'stable';
}

/** `true` when `version` matches the user's skipped version. */
export function isSkippedVersion(
  version: string | null | undefined,
  skipped: string | null | undefined,
): boolean {
  const target = normalizeVersion(version);
  const ignored = normalizeVersion(skipped);
  return target.length > 0 && target === ignored;
}

/** Whether the status bar should display a subtle update badge. */
export function shouldShowUpdateBadge(
  phase: UpdatePhase,
  version: string | null | undefined,
  skipped: string | null | undefined,
): boolean {
  if (phase === 'ready' || phase === 'downloading' || phase === 'installing') return true;
  if (phase !== 'available') return false;
  const normalized = normalizeVersion(version);
  return normalized.length > 0 && !isSkippedVersion(normalized, skipped);
}

/** Whether a silent startup check should open the update dialog automatically (mobile lacks a status bar). */
export function shouldAutoPromptUpdate(params: {
  isMobile: boolean;
  available: boolean;
  version: string | null | undefined;
  skipped: string | null | undefined;
  lastPromptedVersion: string | null | undefined;
}): boolean {
  if (!params.isMobile || !params.available) return false;
  const target = normalizeVersion(params.version);
  if (!target) return false;
  if (isSkippedVersion(target, params.skipped)) return false;
  return normalizeVersion(params.lastPromptedVersion) !== target;
}

/** Download percentage (0–100), or `null` when total byte size is unknown. */
export function downloadPercent(downloaded: number, total: number | null): number | null {
  if (total === null || !Number.isFinite(total) || total <= 0) return null;
  if (!Number.isFinite(downloaded) || downloaded <= 0) return 0;
  return Math.min(100, Math.max(0, Math.round((downloaded / total) * 100)));
}

/** Formats a byte count into a compact label for progress display. */
export function formatUpdateBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb < 100 ? mb.toFixed(1) : Math.round(mb)} MB`;
  const gb = mb / 1024;
  return `${gb.toFixed(2)} GB`;
}

/** Extracts a short `YYYY-MM-DD` date from an ISO timestamp, or returns empty when unparseable. */
export function formatReleaseDate(iso: string | null | undefined): string {
  if (!iso) return '';
  const match = /^(\d{4}-\d{2}-\d{2})/.exec(iso.trim());
  if (match) return match[1]!;
  const parsed = Date.parse(iso);
  if (Number.isNaN(parsed)) return '';
  return new Date(parsed).toISOString().slice(0, 10);
}
