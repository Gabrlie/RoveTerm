// AI chats, one per host context (this machine, a saved host, a quick-connect address). A chat
// starts its agent on the first message and keeps the conversation while the panel is closed.
import {
  aiAnswer,
  aiCancel,
  aiPrompt,
  aiSetMode,
  aiStart,
  aiAgents,
  aiStop,
  aiRename,
  aiSetOption,
  onAiUpdate,
  type AiSessionSummary,
  type AiRecord,
  type ConfigOption,
  type AiAnswer,
  type AiContext,
  type AiEvent,
  type AiMode,
  type AiOperation,
  type AiOutcome,
  type AgentList,
  type Assessment,
  type PlanEntry,
  type SessionId,
  type ToolCall,
} from '../ipc';
import { errorText, t, type MessageKey } from '../i18n';
import { approvalFor, connectionProblem, itemsFromRecords, titleOf, type ChatItem } from './chatview';
import { prefs } from '../settings/prefs.svelte';

export type { ChatItem } from './chatview';

/** Stable key of a context, to find its chat. */
export function contextKey(context: AiContext): string {
  switch (context.kind) {
    case 'local':
      return 'local';
    case 'host':
      return `host:${context.host_id}`;
    case 'quick':
      return `quick:${context.address}`;
  }
}

export class Chat {
  /** The backend session, once started. */
  id = $state<string | null>(null);
  /** The agent answering (`builtin:<id>` or `acp:<id>`); the default when not chosen. */
  agent = $state<string | null>(null);
  host = $state('');
  /** The agent's name as it introduced itself. */
  agentName = $state<string | null>(null);
  items = $state<ChatItem[]>([]);
  plan = $state<PlanEntry[]>([]);
  starting = $state(false);
  /** A turn is running. */
  busy = $state(false);
  mode = $state<AiMode>('ask_by_risk');
  /** Let the agent read the user's terminal on the host (decided when the agent starts). */
  attachTerminal = $state(true);
  /** The title the user gave; otherwise the first question names the chat. */
  title = $state<string | null>(null);
  /** Settings of the running agent the user can change (model, reasoning effort). */
  options = $state<ConfigOption[]>([]);
  /** Settings chosen before the agent started, applied once it has. */
  wanted = $state<Record<string, string>>({});
  /** An earlier session this chat continues; its id is reused when the agent starts. */
  resumeId = $state<string | null>(null);

  /** What the chat is called: the user's title, or its first question. */
  get name(): string | null {
    if (this.title) return this.title;
    const first = this.items.find((item) => item.kind === 'user');
    return first?.kind === 'user' ? titleOf(first.text) : null;
  }

  constructor(
    readonly key: string,
    readonly context: AiContext,
  ) {
    this.mode = prefs.value.ai.mode;
  }

  /** Approvals still waiting for the user. */
  get waiting(): number {
    return this.items.filter((item) => item.kind === 'approval' && item.answer === undefined).length;
  }

  private last(): ChatItem | undefined {
    return this.items[this.items.length - 1];
  }

  /** The connection row of the latest turn, if it has one. */
  private connection(): Extract<ChatItem, { kind: 'connection' }> | undefined {
    for (let at = this.items.length - 1; at >= 0; at -= 1) {
      const item = this.items[at];
      if (item?.kind === 'user') return undefined;
      if (item?.kind === 'connection') return item;
    }
    return undefined;
  }

  /** The agent went on after a connection problem. */
  private reconnected(): void {
    const open = this.connection();
    if (open?.state === 'retrying') open.state = 'recovered';
  }

  handle(event: AiEvent): void {
    switch (event.event) {
      case 'update': {
        const update = event.update;
        if (update.kind === 'message' && connectionProblem(update.text) && update.text.length < 2000) {
          // A warning the agent passes on while it reconnects, not part of its reply.
          const open = this.connection();
          if (open?.state === 'retrying') {
            open.retries += 1;
            open.detail = update.text.trim();
          } else {
            this.items.push({ kind: 'connection', detail: update.text.trim(), retries: 1, state: 'retrying' });
          }
          break;
        }
        this.reconnected();
        if (update.kind === 'message') {
          const last = this.last();
          if (last?.kind === 'agent' && last.messageId === update.message_id) last.text += update.text;
          else this.items.push({ kind: 'agent', text: update.text, messageId: update.message_id });
        } else if (update.kind === 'thought') {
          const last = this.last();
          if (last?.kind === 'thought') last.text += update.text;
          else this.items.push({ kind: 'thought', text: update.text });
        } else if (update.kind === 'tool_call') {
          const existing = this.items.find((item): item is Extract<ChatItem, { kind: 'tool' }> => item.kind === 'tool' && item.call.id === update.call.id);
          if (existing) {
            const call = update.call;
            existing.call = {
              ...existing.call,
              title: call.title ?? existing.call.title,
              kind: call.kind ?? existing.call.kind,
              status: call.status ?? existing.call.status,
              content: call.content.length > 0 ? call.content : existing.call.content,
              locations: call.locations.length > 0 ? call.locations : existing.call.locations,
              raw_input: call.raw_input ?? existing.call.raw_input,
            };
          } else {
            this.items.push({ kind: 'tool', call: update.call });
          }
        } else if (update.kind === 'plan') {
          this.plan = update.entries;
        } else if (update.kind === 'config_options') {
          this.options = update.options.filter((option) => option.category !== 'mode');
        }
        break;
      }
      case 'approval':
        this.items.push({
          kind: 'approval',
          requestId: event.request_id,
          operation: event.operation,
          assessment: event.assessment,
          rememberable: event.rememberable,
          agentTool: event.agent_tool,
          answer: undefined,
          outcome: null,
        });
        break;
      case 'approved': {
        const item = this.items.find((entry) => entry.kind === 'approval' && entry.requestId === event.request_id);
        if (item?.kind === 'approval') item.answer = event.answer;
        break;
      }
      case 'executed':
      {
        // The result of an approved operation completes its approval's row.
        const approval = approvalFor(this.items, event.operation);
        if (approval) approval.outcome = event.outcome;
        // The agent's own tools show as tool items; operations here are RoveTerm's.
        else if (event.operation.op !== 'agent_tool') {
          this.items.push({ kind: 'operation', operation: event.operation, assessment: event.assessment, approved: event.approved, outcome: event.outcome });
        }
        break;
      }
    }
  }

  private notice(error: unknown): void {
    const text = errorText(error);
    if (connectionProblem(text)) {
      const open = this.connection();
      if (open) {
        open.state = 'failed';
        open.detail = text;
      } else {
        this.items.push({ kind: 'connection', detail: text, retries: 0, state: 'failed' });
      }
      return;
    }
    const code = typeof error === 'object' && error !== null && 'code' in error ? String((error as { code: unknown }).code) : null;
    this.items.push({ kind: 'notice', text, code });
  }

  /** Sends the last message again, after the connection failed. */
  async retry(terminal: SessionId | null): Promise<void> {
    const last = [...this.items].reverse().find((item) => item.kind === 'user');
    if (last?.kind !== 'user') return;
    const open = this.connection();
    if (open) {
      open.state = 'retrying';
      open.retries = 0;
    }
    await this.send(last.text, terminal, true);
  }

  /** Sends `text`, starting the agent first when needed. `terminal` is the user's terminal on the
   *  host; `again` sends the last message once more without repeating it in the conversation. */
  async send(text: string, terminal: SessionId | null, again = false): Promise<void> {
    if (this.busy || this.starting || !text.trim()) return;
    if (!again) this.items.push({ kind: 'user', text });
    if (!this.id) {
      this.starting = true;
      try {
        const agent = this.agent ?? ai.defaultAgent;
        if (!agent) throw { code: 'ai.no_agent', message: 'no agent' };
        this.agent = agent;
        const started = await aiStart(this.context, agent, this.attachTerminal ? terminal : null, this.resumeId);
        this.id = started.id;
        this.resumeId = null;
        this.host = started.host;
        this.agentName = started.agent;
        this.options = started.config_options;
        ai.byId[started.id] = this;
        if (this.mode !== prefs.value.ai.mode) await aiSetMode(started.id, this.mode);
        for (const [option, value] of Object.entries(this.wanted)) {
          if (this.options.some((entry) => entry.id === option && entry.current !== value)) this.options = await aiSetOption(started.id, option, value);
        }
        this.wanted = {};
        if (this.title) await aiRename(started.id, this.title);
      } catch (error) {
        this.notice(error);
        return;
      } finally {
        this.starting = false;
      }
    }
    const id = this.id;
    if (!id) return;
    this.busy = true;
    try {
      const reason = await aiPrompt(id, text);
      if (reason !== 'end_turn' && reason !== 'cancelled') {
        const key = `ai.stop.${reason}` as MessageKey;
        this.items.push({ kind: 'notice', text: t(key) === key ? reason : t(key), code: null });
      }
    } catch (error) {
      this.notice(error);
    } finally {
      this.busy = false;
    }
  }

  async cancel(): Promise<void> {
    if (this.id) await aiCancel(this.id).catch((error: unknown) => this.notice(error));
  }

  async answer(requestId: number, answer: AiAnswer): Promise<void> {
    if (this.id) await aiAnswer(this.id, requestId, answer).catch((error: unknown) => this.notice(error));
  }

  /** The value of setting `option`: chosen, or as the agent reports it. */
  optionValue(option: ConfigOption): string {
    return this.wanted[option.id] ?? option.current;
  }

  /** Sets one of the agent's settings, or remembers it until the agent starts. */
  async setOption(option: string, value: string): Promise<void> {
    if (!this.id) {
      this.wanted = { ...this.wanted, [option]: value };
      return;
    }
    try {
      this.options = await aiSetOption(this.id, option, value);
    } catch (error) {
      this.notice(error);
    }
  }

  /** Names the chat (an empty title goes back to the first question). */
  async rename(title: string): Promise<void> {
    const trimmed = title.trim();
    if (!trimmed) {
      this.title = null;
      return;
    }
    this.title = trimmed;
    const id = this.id ?? this.resumeId;
    if (id) this.title = await aiRename(id, trimmed).catch((error: unknown) => (this.notice(error), trimmed));
  }

  async setMode(mode: AiMode): Promise<void> {
    this.mode = mode;
    if (this.id) await aiSetMode(this.id, mode).catch((error: unknown) => this.notice(error));
  }

  /** Ends the agent; the chat can start again. */
  async stop(): Promise<void> {
    const id = this.id;
    this.id = null;
    this.busy = false;
    if (id) {
      delete ai.byId[id];
      await aiStop(id).catch((error: unknown) => console.warn('stopping the agent failed', error));
    }
  }
}

class AiStore {
  /** Not reactive itself, so looking up a chat from a derived value may create it; `version`
   *  changes when a chat is replaced. */
  private chats = new Map<string, Chat>();
  version = $state(0);
  /** Chats by backend session id, for routing events. */
  byId: Record<string, Chat> = {};
  /** The agents of this device; `null` until listed. */
  agents = $state<AgentList | null>(null);
  private listening = false;

  /** Approvals waiting for the user, across all conversations. */
  get waiting(): number {
    void this.version;
    let count = 0;
    for (const chat of this.chats.values()) count += chat.waiting;
    return count;
  }

  chat(context: AiContext): Chat {
    void this.version;
    const key = contextKey(context);
    let chat = this.chats.get(key);
    if (!chat) {
      chat = new Chat(key, context);
      this.chats.set(key, chat);
    }
    return chat;
  }

  /** Starts a new conversation for `chat`'s context, with `agent` when given. */
  async reset(chat: Chat, agent: string | null = chat.agent): Promise<void> {
    await chat.stop();
    const next = new Chat(chat.key, chat.context);
    next.agent = agent;
    next.mode = chat.mode;
    next.attachTerminal = chat.attachTerminal;
    this.chats.set(chat.key, next);
    this.version += 1;
  }

  /** Opens earlier session `summary` (with its `records`) as the chat of its host: it is shown
   *  again and goes on with the same agent at the next message. */
  async resume(summary: AiSessionSummary, records: AiRecord[]): Promise<void> {
    if (!summary.context || !summary.agent_ref) return;
    const key = contextKey(summary.context);
    const current = this.chats.get(key);
    if (current?.id === summary.id) return;
    if (current) await current.stop();
    const chat = new Chat(key, summary.context);
    chat.agent = summary.agent_ref;
    chat.resumeId = summary.id;
    chat.title = summary.title;
    chat.items = itemsFromRecords(records);
    this.chats.set(key, chat);
    this.version += 1;
  }

  /** Switches `chat` to `agent`: the conversation starts over when an agent was already talking. */
  async switchAgent(chat: Chat, agent: string): Promise<void> {
    if (chat.agent === agent) return;
    if (chat.id || chat.items.length > 0) await this.reset(chat, agent);
    else chat.agent = agent;
  }

  async refreshAgents(): Promise<void> {
    this.agents = await aiAgents();
  }

  /** The agent new conversations use: the default when it is available, else the first. */
  get defaultAgent(): string | null {
    const list = this.agents;
    if (!list) return null;
    const usable = list.agents.filter((agent) => agent.enabled);
    return usable.some((agent) => agent.agent === list.default) ? list.default : (usable[0]?.agent ?? null);
  }

  /** Routes backend events to their chats; called once. */
  listen(): void {
    if (this.listening) return;
    this.listening = true;
    onAiUpdate((id, event) => this.byId[id]?.handle(event)).catch((error: unknown) => console.warn('AI events unavailable', error));
  }
}

export const ai = new AiStore();
