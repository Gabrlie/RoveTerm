// Pure helpers for the file browser: POSIX paths, sizes, permissions and ordering.
import type { DirEntry } from '../ipc';

/** Joins a remote directory and a name: `('/', 'a')` → `/a`. */
export function joinPath(dir: string, name: string): string {
  return dir.endsWith('/') ? `${dir}${name}` : `${dir}/${name}`;
}

/** Parent of a remote path, or null at the root: `/a/b` → `/a`, `/a` → `/`. */
export function parentPath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, '');
  if (trimmed === '') return null;
  const index = trimmed.lastIndexOf('/');
  if (index < 0) return null;
  return index === 0 ? '/' : trimmed.slice(0, index);
}

/** Normalizes a typed path: collapses repeated slashes, resolves `.` and `..`. */
export function normalizePath(path: string): string {
  const parts: string[] = [];
  for (const part of path.split('/')) {
    if (part === '' || part === '.') continue;
    if (part === '..') parts.pop();
    else parts.push(part);
  }
  return `/${parts.join('/')}`;
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** Formats a byte count with one decimal above 1 KB, such as `1.5 MB`. */
export function formatSize(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${UNITS[unit]}`;
}

/** Formats permission bits like `ls -l`: `rwxr-xr-x`. */
export function formatMode(mode: number | null): string {
  if (mode === null) return '';
  const bits = 'rwxrwxrwx';
  let text = '';
  for (let i = 0; i < 9; i += 1) text += mode & (1 << (8 - i)) ? bits[i] : '-';
  return text;
}

/** Formats Unix seconds in the user's locale. */
export function formatTime(seconds: number | null): string {
  if (seconds === null) return '';
  return new Date(seconds * 1000).toLocaleString(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/** Directories first, then names in natural, case-insensitive order. */
export function sortEntries(entries: DirEntry[]): DirEntry[] {
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
  return [...entries].sort((a, b) => {
    const dirA = a.kind === 'dir' ? 0 : 1;
    const dirB = b.kind === 'dir' ? 0 : 1;
    return dirA - dirB || collator.compare(a.name, b.name);
  });
}

/** Last component of a local or remote path. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return index < 0 ? trimmed : trimmed.slice(index + 1);
}
