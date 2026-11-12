// What an AI conversation shows: its items as the backend reported them, and how they are laid
// out — an approval and the result of the operation it allowed are one row, and operations that
// ran by themselves (read-only, without approval) fold into one line when several follow each other.
import type { AiAnswer, AiOperation, AiOutcome, AiRecord, Assessment, ReviewNote, Risk, ToolCall } from '../ipc';

/** Text that reports a lost or refused connection to the model service (as Codex warns while it
 *  retries, or as a turn fails), rather than a reply. */
export function connectionProblem(text: string): boolean {
  return /stream disconnected|falling back from websockets|reconnecting\b|connection refused|actively refused|积极拒绝|os error (10061|10060|111|61)\b|econnrefused|error sending request|failed to connect|connection reset|timed out waiting/i.test(text);
}

/** A title from a question: its first line, at most 60 characters. */
export function titleOf(text: string): string {
  const line = text.split('\n').map((part) => part.trim()).find((part) => part.length > 0) ?? '';
  const chars = [...line];
  return chars.length > 60 ? `${chars.slice(0, 60).join('')}…` : line;
}

/** The command line in an agent tool's raw input: `{"command": "…"}`, or a word list where
 *  `bash -lc "<script>"` shows as the script. */
export function commandOf(rawInput: unknown): string | null {
  if (typeof rawInput !== 'object' || rawInput === null) return null;
  const input = rawInput as Record<string, unknown>;
  const command = input.command ?? input.cmd;
  if (typeof command === 'string') return command;
  if (!Array.isArray(command) || !command.every((word) => typeof word === 'string')) return null;
  const words = command as string[];
  const shell = words[0]?.split('/').pop() ?? '';
  if (words.length === 3 && ['sh', 'bash', 'zsh', 'dash'].includes(shell) && ['-c', '-lc', '-ic', '-lic'].includes(words[1] ?? '')) return words[2] ?? null;
  return words.map((word) => (/^[\w@%+=:,./-]+$/.test(word) ? word : `'${word.replace(/'/g, `'\\''`)}'`)).join(' ');
}

/** What an agent's own tool call acts on: its command, else the first file it named. */
export function toolSubject(call: ToolCall): string {
  return commandOf(call.raw_input) ?? call.locations[0] ?? '';
}

export type ChatItem =
  | { kind: 'user'; text: string }
  | { kind: 'agent'; text: string; messageId: string | null }
  | { kind: 'thought'; text: string }
  /** One of an ACP agent's own tools, as it reported it. */
  | { kind: 'tool'; call: ToolCall }
  /** An operation that needed the user, and later its result. */
  | {
      kind: 'approval';
      requestId: number;
      operation: AiOperation;
      assessment: Assessment;
      rememberable: boolean;
      agentTool: boolean;
      /** In the auto mode, why the judge did not let it run. */
      review: ReviewNote | null;
      /** `undefined` while waiting; `null` when withdrawn. */
      answer: AiAnswer | null | undefined;
      outcome: AiOutcome | null;
    }
  /** An operation that ran (or was refused) without asking; `reviewer` names the reviewer that
   *  let it run. */
  | { kind: 'operation'; operation: AiOperation; assessment: Assessment; approved: boolean; outcome: AiOutcome; reviewer?: string }
  | { kind: 'notice'; text: string; code: string | null }
  /** The agent lost its connection to the model service: retrying, given up, or back. */
  | { kind: 'connection'; detail: string; retries: number; state: 'retrying' | 'failed' | 'recovered' }
  /** The earlier conversation was summarized to stay within the context window. */
  | { kind: 'compacted'; summary: string }
  /** The fleet lead handed `task` to sub-agent conversation `id` on `host`. */
  | { kind: 'subtask'; id: string; host: string; task: string };

export type OperationItem = Extract<ChatItem, { kind: 'operation' }>;
export type ApprovalItem = Extract<ChatItem, { kind: 'approval' }>;

export type Row =
  | { kind: 'item'; item: ChatItem; key: number }
  /** Two or more operations in a row that ran by themselves and succeeded. */
  | { kind: 'group'; items: OperationItem[]; key: number };

function sameOperation(a: AiOperation, b: AiOperation): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Where the result of `operation` belongs: the latest allowed approval of it still without one. */
export function approvalFor(items: ChatItem[], operation: AiOperation): ApprovalItem | undefined {
  for (let at = items.length - 1; at >= 0; at -= 1) {
    const item = items[at];
    if (item?.kind === 'approval' && item.outcome === null && item.answer !== undefined && sameOperation(item.operation, operation)) return item;
  }
  return undefined;
}

/** The approval an answer belongs to: the latest one with `requestId` still waiting. A continued
 *  session numbers its requests from 1 again, so older rows with the same number are skipped. */
export function waitingApproval(items: ChatItem[], requestId: number): ApprovalItem | undefined {
  for (let at = items.length - 1; at >= 0; at -= 1) {
    const item = items[at];
    if (item?.kind === 'approval' && item.requestId === requestId && item.answer === undefined) return item;
  }
  return undefined;
}

/** An operation that ran by itself and succeeded, which may fold with its neighbours. */
function quiet(item: ChatItem): item is OperationItem {
  // Only reads fold together: a change that ran without asking (auto mode, a reviewer) stays
  // on its own row.
  if (item.kind !== 'operation' || item.approved || item.reviewer || item.assessment.risk !== 'read_only') return false;
  const outcome = item.outcome;
  if (outcome.result === 'denied' || outcome.result === 'failed') return false;
  return outcome.result !== 'command' || outcome.output.exit_code === 0;
}

/** An allowed approval of an agent's own tool that the agent also reported as a tool call: its
 *  tool row already shows it, with the result. */
function shownAsTool(item: ChatItem, titles: Set<string>): boolean {
  return item.kind === 'approval' && item.operation.op === 'agent_tool' && (item.answer === 'allow_once' || item.answer === 'allow_session') && titles.has(item.operation.title);
}

export function rows(items: ChatItem[]): Row[] {
  const titles = new Set(items.flatMap((item) => (item.kind === 'tool' && item.call.title ? [item.call.title] : [])));
  const out: Row[] = [];
  let run: OperationItem[] = [];
  let runStart = 0;
  const flush = (): void => {
    if (run.length >= 2) out.push({ kind: 'group', items: run, key: runStart });
    else if (run.length === 1) out.push({ kind: 'item', item: run[0] as OperationItem, key: runStart });
    run = [];
  };
  items.forEach((item, at) => {
    if (shownAsTool(item, titles)) return;
    if (quiet(item)) {
      if (run.length === 0) runStart = at;
      run.push(item);
      return;
    }
    flush();
    out.push({ kind: 'item', item, key: at });
  });
  flush();
  return out;
}

/** The thing an operation acts on: a command, a path, or a tool's title. */
export function subject(operation: AiOperation): string {
  switch (operation.op) {
    case 'command':
      return operation.command;
    case 'read_file':
    case 'write_file':
    case 'edit_file':
    case 'list_dir':
      return operation.path;
    case 'read_terminal':
    case 'read_notes':
    case 'update_notes':
      return '';
    case 'agent_tool':
      return operation.command ?? operation.path ?? '';
  }
}

/** How a row's operation ended. */
export function outcomeState(outcome: AiOutcome | null): 'running' | 'done' | 'failed' | 'denied' {
  if (!outcome) return 'running';
  if (outcome.result === 'denied') return 'denied';
  if (outcome.result === 'failed') return 'failed';
  if (outcome.result === 'command' && outcome.output.exit_code !== 0) return 'failed';
  return 'done';
}

function assessed(risk: unknown): Assessment {
  const level: Risk = risk === 'modify' || risk === 'dangerous' ? risk : 'read_only';
  return { risk: level, programs: [], opaque: false, reasons: [] };
}

/** An outcome as the journal keeps it (a command's output flattened) as the live one. */
function outcomeOf(value: unknown): AiOutcome {
  const outcome = (typeof value === 'object' && value !== null ? value : {}) as Record<string, unknown>;
  if (outcome.result === 'command') {
    const text = (key: string): string => (typeof outcome[key] === 'string' ? (outcome[key] as string) : '');
    return {
      result: 'command',
      output: { stdout: text('stdout'), stderr: text('stderr'), exit_code: typeof outcome.exit_code === 'number' ? outcome.exit_code : null, truncated: outcome.truncated === true },
    };
  }
  return outcome as unknown as AiOutcome;
}

/** The conversation of an earlier session from its journal records (oldest first), to show it
 *  again when it is resumed. Approvals left unanswered count as withdrawn. */
export function itemsFromRecords(records: AiRecord[]): ChatItem[] {
  const items: ChatItem[] = [];
  const text = (record: AiRecord): string => (typeof record.payload.text === 'string' ? record.payload.text : '');
  /** The reviewer that let the next operation run. */
  let reviewed: { operation: string; reviewer: string } | null = null;
  for (const record of records) {
    const payload = record.payload;
    switch (record.kind) {
      case 'ai/prompt':
        items.push({ kind: 'user', text: text(record) });
        break;
      case 'ai/message':
        // Warnings an agent passed on while reconnecting were recorded as its reply.
        if (connectionProblem(text(record)) && text(record).length < 4000) items.push({ kind: 'connection', detail: text(record), retries: 1, state: 'retrying' });
        else items.push({ kind: 'agent', text: text(record), messageId: null });
        break;
      case 'ai/thought':
        items.push({ kind: 'thought', text: text(record) });
        break;
      case 'ai/approval':
        if ('asked' in payload) {
          items.push({
            kind: 'approval',
            requestId: Number(payload.request_id),
            operation: payload.asked as AiOperation,
            assessment: { ...assessed(payload.risk), reasons: Array.isArray(payload.reasons) ? (payload.reasons as Assessment['reasons']) : [] },
            rememberable: false,
            agentTool: payload.agent_tool === true,
            review: (payload.review as ReviewNote | null | undefined) ?? null,
            answer: undefined,
            outcome: null,
          });
        } else {
          const asked = items.find((item) => item.kind === 'approval' && item.requestId === Number(payload.request_id));
          if (asked?.kind === 'approval') asked.answer = (payload.answer as AiAnswer | null) ?? null;
        }
        break;
      case 'ai/command':
      case 'ai/file_read':
      case 'ai/file_write':
      case 'ai/list_dir':
      case 'ai/terminal_read':
      case 'ai/notes_read':
      case 'ai/notes_write':
      case 'ai/tool_call': {
        if (!('operation' in payload)) {
          // One of the agent's own tools, as it reported it.
          if (record.kind === 'ai/tool_call') {
            const call: ToolCall = {
              id: String(payload.id ?? ''),
              title: typeof payload.title === 'string' ? payload.title : null,
              kind: typeof payload.kind === 'string' ? payload.kind : null,
              status: typeof payload.status === 'string' ? payload.status : null,
              content: [],
              locations: Array.isArray(payload.locations) ? (payload.locations as string[]) : [],
              raw_input: typeof payload.input === 'string' ? safeJson(payload.input) : null,
            };
            items.push({ kind: 'tool', call });
          }
          break;
        }
        const operation = payload.operation as AiOperation;
        const outcome = outcomeOf(payload.outcome);
        const approval = approvalFor(items, operation);
        if (approval) approval.outcome = outcome;
        else if (operation.op !== 'agent_tool') {
          const reviewer = reviewed?.operation === JSON.stringify(operation) ? reviewed.reviewer : undefined;
          items.push({ kind: 'operation', operation, assessment: assessed(payload.risk), approved: payload.approved === true, outcome, reviewer });
        }
        reviewed = null;
        break;
      }
      case 'ai/reviewed':
        reviewed = payload.allowed === true && typeof payload.reviewer === 'string' ? { operation: JSON.stringify(payload.operation), reviewer: payload.reviewer } : null;
        break;
      case 'ai/compacted':
        items.push({ kind: 'compacted', summary: typeof payload.summary === 'string' ? payload.summary : '' });
        break;
      case 'ai/turn_ended': {
        const reason = typeof payload.stop_reason === 'string' ? payload.stop_reason : '';
        if (!reason.startsWith('error: ')) break;
        const detail = reason.slice('error: '.length);
        const open = [...items].reverse().find((item) => item.kind === 'connection');
        if (connectionProblem(detail)) {
          if (open?.kind === 'connection' && open.state === 'retrying') {
            open.state = 'failed';
            open.detail = detail;
          } else {
            items.push({ kind: 'connection', detail, retries: 0, state: 'failed' });
          }
        } else {
          items.push({ kind: 'notice', text: detail, code: null });
        }
        break;
      }
    }
  }
  // Settled, and numbered apart from the requests of a continued session.
  for (const item of items) {
    if (item.kind !== 'approval') continue;
    if (item.answer === undefined) item.answer = null;
    item.requestId = -1;
  }
  return items;
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}
