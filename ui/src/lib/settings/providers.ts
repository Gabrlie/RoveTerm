// Model providers of the built-in assistant: presets that fill in the form, and ids for new ones.
import type { ProviderFormat, ProviderPrefs } from '../ipc';

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
  { id: 'anthropic', name: 'Anthropic', format: 'anthropic', base_url: 'https://api.anthropic.com/v1', modelHint: 'claude-sonnet-4-5', keyless: false },
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
    max_tokens: null,
    effort: null,
  };
}

/** A custom ACP agent id (`custom-…`) unused in `taken`. */
export function customAgentId(name: string, taken: readonly string[]): string {
  return providerId(`custom-${name}`, taken);
}
