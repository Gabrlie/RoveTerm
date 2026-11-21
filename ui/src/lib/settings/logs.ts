// Pure helpers for parsing, filtering and formatting application log lines.

export type LogSeverity = 'ERROR' | 'WARN' | 'INFO' | 'DEBUG' | 'TRACE' | 'OTHER';
export type LogLevelFilter = 'ALL' | 'ERROR' | 'WARN' | 'INFO' | 'DEBUG';

export interface LogEntry {
  index: number;
  raw: string;
  level: LogSeverity;
}

const LEVEL_RE = /^\s*(?:\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?Z?\s+)?(ERROR|WARN|INFO|DEBUG|TRACE)\b/;

/** Splits log text into indexed entries with detected severity; continuation lines keep the
 *  severity of the preceding log event so stack traces stay visible under level filters. */
export function parseLogLines(content: string): LogEntry[] {
  if (!content) return [];
  const lines = content.split(/\r?\n/);
  const entries: LogEntry[] = [];
  let current: LogSeverity = 'OTHER';
  for (let i = 0; i < lines.length; i += 1) {
    const raw = lines[i] ?? '';
    if (i === lines.length - 1 && raw === '') continue;
    const match = LEVEL_RE.exec(raw);
    if (match) {
      current = match[1] as LogSeverity;
    } else if (!raw.trim()) {
      current = 'OTHER';
    }
    entries.push({ index: i + 1, raw, level: current });
  }
  return entries;
}

/** Filters parsed entries by level and case-insensitive substring query. */
export function filterLogEntries(entries: LogEntry[], level: LogLevelFilter, query: string): LogEntry[] {
  const needle = query.trim().toLowerCase();
  return entries.filter((entry) => {
    if (level !== 'ALL') {
      if (level === 'DEBUG') {
        if (entry.level !== 'DEBUG' && entry.level !== 'TRACE') return false;
      } else if (entry.level !== level) {
        return false;
      }
    }
    if (needle && !entry.raw.toLowerCase().includes(needle)) return false;
    return true;
  });
}

/** Formats a byte size into a compact human-readable label. */
export function formatLogSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
