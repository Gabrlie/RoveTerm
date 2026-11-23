// Pure helpers for parsing, filtering and formatting application log lines.

export type LogSeverity = 'ERROR' | 'WARN' | 'INFO' | 'DEBUG' | 'TRACE' | 'OTHER';
export type LogLevelFilter = 'ALL' | 'ERROR' | 'WARN' | 'INFO' | 'DEBUG';

export interface LogEntry {
  index: number;
  raw: string;
  level: LogSeverity;
  timestamp: string | null;
  shortTime: string | null;
  target: string | null;
  message: string;
  continuation: boolean;
}

const ANSI_CSI_RE = /\u001b\[[0-9;?]*[ -/]*[@-~]/g;
const TIMESTAMP_RE = /^\s*(\d{4}-\d{2}-\d{2}[T ](\d{2}:\d{2}:\d{2})(?:[.,](\d+))?(?:Z|[+-]\d{2}(?::?\d{2})?)?)(?:\s+|$)/;
const LEVEL_RE = /^\s*(ERROR|WARN|INFO|DEBUG|TRACE)\b[ \t]*/;
const TARGET_RE = /^([\w.-]+(?:::[\w.-]+)*):(?:[ \t]+|$)/;

/** Splits log text into indexed entries with detected severity; continuation lines keep the
 *  severity of the preceding log event so stack traces stay visible under level filters. */
export function parseLogLines(content: string): LogEntry[] {
  if (!content) return [];
  const lines = content.split(/\r?\n/);
  const entries: LogEntry[] = [];
  let current: LogSeverity = 'OTHER';
  for (let i = 0; i < lines.length; i += 1) {
    const raw = (lines[i] ?? '').replace(ANSI_CSI_RE, '');
    if (i === lines.length - 1 && raw === '') continue;
    const time = TIMESTAMP_RE.exec(raw);
    const body = time ? raw.slice(time[0].length) : raw;
    const severity = LEVEL_RE.exec(body);
    if (severity) current = severity[1] as LogSeverity;
    else if (!raw.trim()) current = 'OTHER';

    const detail = severity ? body.slice(severity[0].length) : raw;
    const target = severity ? TARGET_RE.exec(detail) : null;
    entries.push({
      index: i + 1,
      raw,
      level: current,
      timestamp: time?.[1] ?? null,
      shortTime: time ? time[2] + '.' + (time[3] ?? '').padEnd(3, '0').slice(0, 3) : null,
      target: target?.[1] ?? null,
      message: target ? detail.slice(target[0].length) : detail,
      continuation: !severity && current !== 'OTHER',
    });
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
