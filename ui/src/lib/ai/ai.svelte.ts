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
  onAiUpdate,
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
import { approvalFor, type ChatItem } from './chatview';
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

  handle(event: AiEvent): void {
    switch (event.event) {
      case 'update': {
        const update = event.update;
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
    const code = typeof error === 'object' && error !== null && 'code' in error ? String((error as { code: unknown }).code) : null;
    this.items.push({ kind: 'notice', text: errorText(error), code });
  }

  /** Sends `text`, starting the agent first when needed. `terminal` is the user's terminal on the host. */
  async send(text: string, terminal: SessionId | null): Promise<void> {
    if (this.busy || this.starting || !text.trim()) return;
    this.items.push({ kind: 'user', text });
    if (!this.id) {
      this.starting = true;
      try {
        const agent = this.agent ?? ai.defaultAgent;
        if (!agent) throw { code: 'ai.no_agent', message: 'no agent' };
        this.agent = agent;
        const started = await aiStart(this.context, agent, this.attachTerminal ? terminal : null);
        this.id = started.id;
        this.host = started.host;
        this.agentName = started.agent;
        if (this.mode !== prefs.value.ai.mode) await aiSetMode(started.id, this.mode);
        ai.byId[started.id] = this;
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
    return list.agents.some((agent) => agent.agent === list.default) ? list.default : (list.agents[0]?.agent ?? null);
  }

  /** Routes backend events to their chats; called once. */
  listen(): void {
    if (this.listening) return;
    this.listening = true;
    onAiUpdate((id, event) => this.byId[id]?.handle(event)).catch((error: unknown) => console.warn('AI events unavailable', error));
  }
}

export const ai = new AiStore();
