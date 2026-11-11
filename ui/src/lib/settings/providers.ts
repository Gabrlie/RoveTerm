// Model providers of the built-in assistant: presets that fill in the form, and ids for new ones.
import type { ProviderFormat, ProviderPrefs, ReasoningEffort } from '../ipc';

/** Reasoning effort levels, least first. */
export const EFFORT_LEVELS: readonly ReasoningEffort[] = ['minimal', 'low', 'medium', 'high', 'xhigh', 'max'];

export interface ProviderPreset {
  id: string;
  name: string;
  format: ProviderFormat;
  base_url: string;
  /** Shown as the model field's placeholder; the user picks the model. */
  modelHint: string;
  /** The service takes no API key (a model server on this computer). */
  keyless: boolean;
}

export const PRESETS: ProviderPreset[] = [
  { id: 'openai', name: 'OpenAI', format: 'responses', base_url: 'https://api.openai.com/v1', modelHint: 'gpt-5', keyless: false },
  { id: 'anthropic', name: 'Anthropic', format: 'anthropic', base_url: 'https://api.anthropic.com/v1', modelHint: 'claude-opus-5-5', keyless: false },
  { id: 'deepseek', name: 'DeepSeek', format: 'chat', base_url: 'https://api.deepseek.com/v1', modelHint: 'deepseek-chat', keyless: false },
  { id: 'openrouter', name: 'OpenRouter', format: 'chat', base_url: 'https://openrouter.ai/api/v1', modelHint: 'openai/gpt-5', keyless: false },
  { id: 'ollama', name: 'Ollama', format: 'chat', base_url: 'http://localhost:11434/v1', modelHint: 'qwen3', keyless: true },
  { id: 'custom', name: '', format: 'chat', base_url: '', modelHint: '', keyless: false },
];

/** A provider id (lowercase letters, digits and `-`, at most 40) from `name`, unused in `taken`. */
export function providerId(name: string, taken: readonly string[]): string {
  const base =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 32)
      .replace(/-+$/, '') || 'provider';
  if (!taken.includes(base)) return base;
  for (let n = 2; ; n += 1) {
    const id = `${base}-${n}`;
    if (!taken.includes(id)) return id;
  }
}

/** A new provider's form, filled from `preset`. */
export function fromPreset(preset: ProviderPreset, taken: readonly string[]): ProviderPrefs {
  return {
    id: providerId(preset.name || 'custom', taken),
    name: preset.name,
    format: preset.format,
    base_url: preset.base_url,
    model: '',
    models: [],
    max_tokens: null,
    effort: 'medium',
  };
}

/** A custom ACP agent id (`custom-…`) unused in `taken`. */
export function customAgentId(name: string, taken: readonly string[]): string {
  return providerId(`custom-${name}`, taken);
}

/** The checked models as a provider's `model` (the starred one, else the first checked) and
 *  its other `models`. */
export function providerModels(picked: readonly string[], star: string): { model: string; models: string[] } {
  const unique = [...new Set(picked.map((model) => model.trim()).filter((model) => model.length > 0))];
  const model = unique.includes(star) ? star : (unique[0] ?? '');
  return { model, models: unique.filter((entry) => entry !== model) };
}

/** A fetched model list with the already-checked models kept (they may not be listed). */
export function mergeListed(fetched: readonly string[], picked: readonly string[]): string[] {
  return [...new Set([...picked, ...fetched])].sort((a, b) => Number(picked.includes(b)) - Number(picked.includes(a)) || a.localeCompare(b));
}
