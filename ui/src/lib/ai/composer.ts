// The input card's settings and `/` menu: which of an agent's settings the model popover shows
// (model, reasoning effort, switches such as fast mode), and what the `/` menu lists (the
// agent's commands and its other settings).
import type { AgentCommand, ConfigOption } from '../ipc';
import type { MessageKey } from '../i18n';

/** An agent's settings, by where the input card shows them. */
export interface SplitOptions {
  model: ConfigOption | null;
  effort: ConfigOption | null;
  /** On/off settings, shown as switches in the model popover (Claude Code's fast mode). */
  toggles: ConfigOption[];
  /** A choice of speed with a fast tier (Codex's `service_tier`), switched in the model popover;
   *  its other tiers stay in the `/` menu. */
  speed: ConfigOption | null;
  /** Everything else, listed in the `/` menu. */
  others: ConfigOption[];
}

export function splitOptions(options: readonly ConfigOption[]): SplitOptions {
  const model = options.find((option) => option.category === 'model' && option.kind === 'select') ?? null;
  const effort =
    options.find((option) => option.category === 'thought_level' && option.kind === 'select') ??
    options.find((option) => option !== model && option.kind === 'select' && /effort|reasoning|thought/i.test(option.id)) ??
    null;
  const toggles = options.filter((option) => option.kind === 'boolean');
  const speed =
    options.find((option) => option.kind === 'select' && option !== model && option !== effort && SPEED.test(`${option.id} ${option.name}`) && fastValue(option) !== null) ?? null;
  const others = options.filter((option) => option !== model && option !== effort && option.kind !== 'boolean');
  return { model, effort, toggles, speed, others };
}

const SPEED = /fast|speed|service[_ -]?tier|priority/i;

/** The fast tier of a speed setting. */
export function fastValue(option: ConfigOption): string | null {
  return option.choices.find((choice) => /^(fast|priority)$/i.test(choice.value) || /^fast\b/i.test(choice.name))?.value ?? null;
}

/** What the fast switch sets: the fast tier when it is off, else the standard tier (`default`,
 *  `standard`, `auto`, else the first other one). */
export function speedToggle(option: ConfigOption, current: string): string {
  const fast = fastValue(option);
  if (fast !== null && current !== fast) return fast;
  const others = option.choices.filter((choice) => choice.value !== fast);
  return (others.find((choice) => /^(default|standard|auto|normal)$/i.test(choice.value)) ?? others[0])?.value ?? current;
}

/** The dictionary entry naming a setting RoveTerm knows, whatever agent offers it. */
export function optionKey(option: ConfigOption): MessageKey | null {
  if (option.category === 'model') return 'ai.model';
  if (option.category === 'thought_level' || /^(effort|reasoning[_-]?effort|thought[_-]?level)$/i.test(option.id)) return 'ai.effort';
  if (SPEED.test(option.id) || /fast/i.test(option.name)) return 'ai.fastMode';
  const known: Record<string, MessageKey> = { sandbox: 'ai.option.sandbox', sandbox_mode: 'ai.option.sandbox', approval: 'ai.option.approval', approval_policy: 'ai.option.approval', personality: 'ai.option.personality' };
  return known[option.id.toLowerCase()] ?? null;
}

const CHOICES: Record<string, MessageKey> = {
  minimal: 'ai.effort.minimal',
  low: 'ai.effort.low',
  medium: 'ai.effort.medium',
  high: 'ai.effort.high',
  xhigh: 'ai.effort.xhigh',
  max: 'ai.effort.max',
  none: 'ai.effort.none',
  fast: 'ai.speed.fast',
  priority: 'ai.speed.fast',
  flex: 'ai.speed.flex',
  default: 'ai.speed.default',
  standard: 'ai.speed.default',
  'read-only': 'ai.sandbox.readOnly',
  'workspace-write': 'ai.sandbox.workspaceWrite',
  'danger-full-access': 'ai.sandbox.fullAccess',
  full: 'ai.sandbox.fullAccess',
};

/** The dictionary entry naming a choice RoveTerm knows (effort levels, speed tiers, sandbox
 *  modes); model names and other choices keep the agent's name. */
export function choiceKey(option: ConfigOption, value: string): MessageKey | null {
  if (option.category === 'model') return null;
  return CHOICES[value.toLowerCase()] ?? null;
}

/** Commands of the built-in assistant, shown in its `/` menu. It has none yet; this is where
 *  RoveTerm's own commands are added. */
export function builtinCommands(_agent: string): AgentCommand[] {
  return [];
}

/** What is typed after `/` while the input is a command name being typed; `null` otherwise. */
export function slashQuery(draft: string): string | null {
  const match = /^\/(\S*)$/.exec(draft);
  return match ? (match[1] ?? '') : null;
}

export type SlashItem = { kind: 'command'; command: AgentCommand } | { kind: 'setting'; option: ConfigOption };

/** The `/` menu for `query`: matching commands (names starting with it first), then matching
 *  settings. */
export function slashItems(query: string, commands: readonly AgentCommand[], settings: readonly ConfigOption[]): SlashItem[] {
  const wanted = query.toLowerCase();
  const rank = (name: string): number => {
    const lower = name.toLowerCase();
    if (lower.startsWith(wanted)) return 0;
    return lower.includes(wanted) ? 1 : -1;
  };
  const found = commands
    .map((command) => ({ command, rank: rank(command.name) }))
    .filter((entry) => entry.rank >= 0)
    .sort((a, b) => a.rank - b.rank)
    .map(({ command }): SlashItem => ({ kind: 'command', command }));
  const matching = settings.filter((option) => rank(option.name) >= 0 || rank(option.id) >= 0).map((option): SlashItem => ({ kind: 'setting', option }));
  return [...found, ...matching];
}

type Choice = ConfigOption['choices'][number];
type OptionValue = (option: ConfigOption) => string;

/** The current choice, keeping an agent's reported value when it has no matching choice. */
export function currentChoice(option: ConfigOption | null, value: OptionValue): Choice | null {
  if (!option) return null;
  const current = value(option).trim();
  if (!current) return null;
  return option.choices.find((choice) => choice.value === current) ?? { value: current, name: current, description: null };
}

export function effortPill(label: string | null): string {
  return label ? `· ${label}` : '';
}

export function fastIndicator(speed: ConfigOption | null, toggles: readonly ConfigOption[], value: OptionValue): 'on' | 'off' | null {
  const fastToggles = toggles.filter((option) => optionKey(option) === 'ai.fastMode');
  if (speed && value(speed) === fastValue(speed)) return 'on';
  if (fastToggles.some((option) => value(option) === 'true')) return 'on';
  return speed || fastToggles.length > 0 ? 'off' : null;
}

/** Groups matching models by provider in the order each group was introduced. */
export function modelGroups(choices: readonly Choice[], query: string): [string, Choice[]][] {
  const wanted = query.trim().toLowerCase();
  const groups = new Map<string, Choice[]>();
  for (const choice of choices) {
    const group = choice.group ?? '';
    if (!`${choice.name} ${group}`.toLowerCase().includes(wanted)) continue;
    const entries = groups.get(group);
    if (entries) entries.push(choice);
    else groups.set(group, [choice]);
  }
  return [...groups];
}

export function canPrestart(
  agent: string | null,
  enabled: boolean,
  chat: { external: string | null; context: { kind: string }; id: string | null; starting: boolean; prestartedAgent: string | null },
): agent is string {
  return enabled && !!agent?.startsWith('acp:') && !chat.external && chat.context.kind !== 'fleet' &&
    !chat.id && !chat.starting && chat.prestartedAgent !== agent;
}

export function canChangeTerminal(chat: { hasUserMessage: boolean; busy: boolean }): boolean {
  return !chat.hasUserMessage && !chat.busy;
}
