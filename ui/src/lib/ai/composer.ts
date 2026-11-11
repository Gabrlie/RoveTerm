// The input card's settings and `/` menu: which of an agent's settings the model popover shows
// (model, reasoning effort, switches such as fast mode), and what the `/` menu lists (the
// agent's commands and its other settings).
import type { AgentCommand, ConfigOption } from '../ipc';

/** An agent's settings, by where the input card shows them. */
export interface SplitOptions {
  model: ConfigOption | null;
  effort: ConfigOption | null;
  /** On/off settings, shown as switches in the model popover (Claude Code's fast mode). */
  toggles: ConfigOption[];
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
  const others = options.filter((option) => option !== model && option !== effort && option.kind !== 'boolean');
  return { model, effort, toggles, others };
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

/** Position on the effort slider of `value` among `option`'s choices (0 when unknown). */
export function effortIndex(option: ConfigOption, value: string): number {
  return Math.max(0, option.choices.findIndex((choice) => choice.value === value));
}
