// Lines of the AI audit view, from journal records (see rove_ai::audit).
import type { AiRecord } from '../ipc';

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
}

function str(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
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
    case 'session_started':
      return { kind, text: str(payload.agent), note: str(payload.mode), detail: '', tone: 'muted' };
    case 'prompt':
      return { kind, text: str(payload.text), note: '', detail: '', tone: 'normal' };
    case 'message':
      return { kind, text: str(payload.text).slice(0, 200), note: '', detail: str(payload.text), tone: 'normal' };
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
      // Agent tools report themselves when finished, without an operation.
      const text = subject(payload.operation) || str(payload.title) || str(payload.input);
      if (!result && payload.status) note = str(payload.status);
      return { kind, text, note, detail, tone };
    }
    case 'turn_ended':
      return { kind, text: '', note: str(payload.stop_reason), detail: '', tone: 'muted' };
    case 'session_ended':
      return { kind, text: '', note: '', detail: '', tone: 'muted' };
    default:
      return null;
  }
}
