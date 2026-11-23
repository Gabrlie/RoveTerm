// Lines of the AI audit view, from journal records (see rove_ai::audit).
import type { AiRecord, AiSessionSummary } from '../ipc';
import { shortHostName } from '../ops/taskview.ts';
import { formatToolOutput, isLeadTool, safeJson, toolSubject } from './chatview.ts';

export interface AuditLine {
  /** Message key suffix under `ai.audit.`, such as `prompt` or `command`. */
  kind: string;
  /** The main text: a prompt, a command, a path. */
  text: string;
  /** A short result: exit code, answer, risk. */
  note: string;
  /** Longer content shown on demand: command output, a reply. */
  detail: string;
  tone: 'normal' | 'muted' | 'warning' | 'danger';
  exitCode?: number | null;
  reviewer?: string;
  confidence?: number;
  toolSubject?: string;
}

function str(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

export type FormattedAuditDetail = { kind: 'diff' | 'text' | 'json'; text: string };

function formatString(text: string, allowScalar = false): FormattedAuditDetail {
  const trimmed = text.trim();
  if (allowScalar || trimmed.startsWith('{') || trimmed.startsWith('[')) {
    const parsed = safeJson(trimmed);
    if (parsed !== null || trimmed === 'null') return { kind: 'json', text: JSON.stringify(parsed, null, 2) };
  }
  return { kind: 'text', text };
}

export function formatAuditDetail(kind: string, payload: unknown): FormattedAuditDetail {
  const normalized = kind.replace(/^ai\//, '');
  if (typeof payload === 'string') return formatString(payload);
  if (payload == null || typeof payload !== 'object') {
    return { kind: 'text', text: payload == null ? '' : String(payload) };
  }

  const obj = record(payload);
  const outcome = record(obj.outcome);
  const operation = record(obj.operation);
  const asked = record(obj.asked);
  const error = str(outcome.error);
  let text = '';

  switch (normalized) {
    case 'command':
      text = [str(outcome.stdout), str(outcome.stderr)].filter(Boolean).join('\n') || error || str(operation.command);
      break;
    case 'file_write':
    case 'notes_write':
      if (str(operation.diff)) return { kind: 'diff', text: str(operation.diff) };
      text = str(outcome.content) || error;
      break;
    case 'file_read':
    case 'terminal_read':
    case 'notes_read':
    case 'list_dir':
      text = str(outcome.content) || error;
      break;
    case 'approval':
    case 'approval_asked':
      if (str(asked.diff)) return { kind: 'diff', text: str(asked.diff) };
      text = str(asked.command) || str(asked.detail);
      break;
    case 'tool_call':
      if (str(outcome.content).trim()) return formatString(str(outcome.content), true);
      if (obj.input != null) {
        return { kind: 'json', text: typeof obj.input === 'string' ? formatToolOutput(obj.input) : JSON.stringify(obj.input, null, 2) };
      }
      break;
    default:
      break;
  }

  text ||= error || str(obj.text) || str(obj.summary);
  if (text) return { kind: 'text', text };
  return { kind: 'json', text: JSON.stringify(payload, null, 2) };
}

export type DateBucket = 'today' | 'yesterday' | 'week' | 'older';

export function groupSessionsByDate(sessions: AiSessionSummary[], now = Date.now()): { bucket: DateBucket; sessions: AiSessionSummary[] }[] {
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const midnight = (daysAgo: number): number => new Date(today.getFullYear(), today.getMonth(), today.getDate() - daysAgo).getTime();
  const yesterday = midnight(1);
  const week = midnight(6);
  const buckets: Record<DateBucket, AiSessionSummary[]> = { today: [], yesterday: [], week: [], older: [] };
  for (const session of sessions) {
    const bucket = session.started >= today.getTime() ? 'today'
      : session.started >= yesterday ? 'yesterday' : session.started >= week ? 'week' : 'older';
    buckets[bucket].push(session);
  }
  const order: DateBucket[] = ['today', 'yesterday', 'week', 'older'];
  return order.filter((bucket) => buckets[bucket].length > 0).map((bucket) => ({ bucket, sessions: buckets[bucket] }));
}

export function filterSessions(sessions: AiSessionSummary[], query: string): AiSessionSummary[] {
  const term = query.trim().toLowerCase();
  if (!term) return sessions;
  return sessions.filter(({ title, host, agent, model, children }) => (
    [title, host, shortHostName(host), agent, model, ...children.flatMap((child) => [child.host, shortHostName(child.host)])]
      .some((value) => value?.toLowerCase().includes(term))
  ));
}

export function formatSessionTime(ts: number, now = Date.now()): string {
  const date = new Date(ts);
  const today = new Date(now);
  const pad = (part: number): string => String(part).padStart(2, '0');
  const day = `${pad(date.getMonth() + 1)}/${pad(date.getDate())}`;
  if (date.getFullYear() !== today.getFullYear()) return `${date.getFullYear()}/${day}`;
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  return date.getMonth() === today.getMonth() && date.getDate() === today.getDate() ? time : `${day} ${time}`;
}

/** The command, path or title an operation is about. */
export function subject(operation: unknown): string {
  const op = record(operation);
  return str(op.command) || str(op.path) || str(op.title) || '';
}

/** Describes a record; `null` for records not shown. */
export function auditLine(entry: AiRecord): AuditLine | null {
  const payload = entry.payload;
  const kind = entry.kind.replace(/^ai\//, '');
  switch (kind) {
    case 'model_changed':
      return { kind, text: [str(payload.provider), str(payload.model)].join(' · '), note: '', detail: '', tone: 'normal' };
    case 'session_started':
      return { kind, text: str(payload.agent), note: str(payload.mode), detail: '', tone: 'muted' };
    case 'prompt':
      return { kind, text: str(payload.text), note: '', detail: '', tone: 'normal' };
    case 'ops_wakeup':
      return { kind, text: '', note: '', detail: str(payload.text), tone: 'muted' };
    case 'message': {
      const detail = str(payload.text);
      const first = detail.split('\n').map((line) => line.trim()).find(Boolean) ?? '';
      const preview = [...first.replace(/^(?:#{1,6}|[-*+])\s+/, '')];
      const text = preview.length > 140 ? `${preview.slice(0, 139).join('')}…` : preview.join('');
      return { kind, text, note: '', detail, tone: 'normal' };
    }
    case 'thought':
      return null;
    case 'approval': {
      if ('asked' in payload) {
        const risk = str(payload.risk);
        return { kind: 'approval_asked', text: subject(payload.asked), note: risk, detail: '', tone: risk === 'dangerous' ? 'danger' : 'warning' };
      }
      return { kind: 'approval_answered', text: '', note: payload.answer === null ? 'withdrawn' : str(payload.answer), detail: '', tone: payload.answer === 'deny' ? 'danger' : 'muted' };
    }
    case 'command':
    case 'file_read':
    case 'file_write':
    case 'list_dir':
    case 'terminal_read':
    case 'notes_read':
    case 'notes_write':
    case 'tool_call': {
      const outcome = record(payload.outcome);
      const result = str(outcome.result);
      let note = result;
      let detail = '';
      let tone: AuditLine['tone'] = 'normal';
      if (result === 'command') {
        note = `exit ${outcome.exit_code ?? '–'}`;
        detail = [str(outcome.stdout), str(outcome.stderr)].filter(Boolean).join('\n');
        if (outcome.exit_code !== 0) tone = 'warning';
      } else if (result === 'denied' || result === 'failed') {
        tone = 'danger';
        detail = str(outcome.error);
      }
      // An edit shows its change.
      const diff = str(record(payload.operation).diff);
      if (diff && !detail) detail = diff;
      const text = subject(payload.operation) || str(payload.title);
      if (!detail && kind === 'tool_call' && payload.input != null) {
        detail = typeof payload.input === 'string' ? payload.input : JSON.stringify(payload.input);
      }
      if (!result && payload.status) note = str(payload.status);
      const line: AuditLine = { kind, text, note, detail, tone };
      if (kind === 'command') line.exitCode = typeof outcome.exit_code === 'number' ? outcome.exit_code : null;
      if (kind === 'tool_call' && isLeadTool(text) && payload.input != null) {
        line.toolSubject = toolSubject({
          id: '', title: text, kind: null, status: null, content: [], locations: [],
          raw_input: typeof payload.input === 'string' ? safeJson(payload.input) : payload.input,
        });
      }
      return line;
    }
    case 'reviewed': {
      const reviewer = str(payload.reviewer);
      const confidence = typeof payload.confidence === 'number' ? Math.round(payload.confidence * 100) : undefined;
      const percent = confidence == null ? '' : ` ${confidence}%`;
      return { kind, text: subject(payload.operation), note: `${reviewer}${percent}`, detail: '', tone: payload.allowed === true ? 'muted' : 'warning', reviewer, confidence };
    }
    case 'compacted':
      return { kind, text: '', note: '', detail: str(payload.summary), tone: 'muted' };
    case 'turn_ended':
      return { kind, text: '', note: str(payload.stop_reason), detail: '', tone: 'muted' };
    case 'session_ended':
      return { kind, text: '', note: '', detail: '', tone: 'muted' };
    default:
      return null;
  }
}
