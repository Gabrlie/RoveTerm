// What an AI conversation shows: its items as the backend reported them, and how they are laid
// out — an approval and the result of the operation it allowed are one row, and operations that
// ran by themselves (read-only, without approval) fold into one line when several follow each other.
import type { AiAnswer, AiOperation, AiOutcome, Assessment, ToolCall } from '../ipc';

/** Text that reports a lost or refused connection to the model service (as Codex warns while it
 *  retries, or as a turn fails), rather than a reply. */
export function connectionProblem(text: string): boolean {
  return /stream disconnected|falling back from websockets|reconnecting\b|connection refused|actively refused|积极拒绝|os error (10061|10060|111|61)\b|econnrefused|error sending request|failed to connect|connection reset|timed out waiting/i.test(text);
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
      /** `undefined` while waiting; `null` when withdrawn. */
      answer: AiAnswer | null | undefined;
      outcome: AiOutcome | null;
    }
  /** An operation that ran (or was refused) without asking. */
  | { kind: 'operation'; operation: AiOperation; assessment: Assessment; approved: boolean; outcome: AiOutcome }
  | { kind: 'notice'; text: string; code: string | null }
  /** The agent lost its connection to the model service: retrying, given up, or back. */
  | { kind: 'connection'; detail: string; retries: number; state: 'retrying' | 'failed' | 'recovered' };

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

/** An operation that ran by itself and succeeded, which may fold with its neighbours. */
function quiet(item: ChatItem): item is OperationItem {
  if (item.kind !== 'operation' || item.approved) return false;
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
    case 'list_dir':
      return operation.path;
    case 'read_terminal':
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
