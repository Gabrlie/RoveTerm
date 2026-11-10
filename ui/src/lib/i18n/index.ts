// Locale selection and message formatting. UI copy always goes through `t`.
import { en, type MessageKey, type Messages } from './en';
import { zhCN } from './zh-CN';
import { isCommandError } from '../ipc';
import { locale } from './locale.svelte';

const catalogs: Record<string, Messages> = { en, 'zh-CN': zhCN };

function pickLocale(): string {
  const preferred = navigator.languages ?? [navigator.language];
  for (const tag of preferred) {
    if (tag.toLowerCase().startsWith('zh')) return 'zh-CN';
    if (tag.toLowerCase().startsWith('en')) return 'en';
  }
  return 'en';
}

locale.messages = catalogs[pickLocale()] ?? en;

export type { MessageKey };

/** Formats `key`, replacing `{name}` placeholders with `params`. A key built at run time that has
 *  no message comes back as itself, so callers can fall back with `t(key) === key`. */
export function t(key: MessageKey, params: Record<string, string | number> = {}): string {
  const template: string | undefined = locale.messages[key] ?? en[key];
  if (template === undefined) return key;
  return template.replace(/\{(\w+)\}/g, (_, name: string) => String(params[name] ?? `{${name}}`));
}

/** Localizes a command error by its code, falling back to the generic message. */
/** Codes whose backend message names the cause (a path, an OS error) and is shown after the text. */
const DETAILED = new Set(['ssh.key', 'ssh.agent', 'ssh.connect', 'ssh.protocol', 'ssh.sftp', 'ssh.proxy', 'ssh.jump', 'ssh.jump_host', 'key.in_use', 'key.install_failed', 'fs.io', 'ai.spawn', 'ai.agent', 'ai.protocol', 'ai.auth_required', 'ai.model', 'input.invalid']);

export function errorText(error: unknown): string {
  if (isCommandError(error)) {
    const key = `error.${error.code}` as MessageKey;
    if (!(key in en)) return `${t('error.unknown')}: ${error.message}`;
    return DETAILED.has(error.code) ? `${t(key)}: ${error.message}` : t(key);
  }
  return `${t('error.unknown')}: ${String(error)}`;
}

/** Switches the language; `auto` follows the system. Text rendered through `t` updates. */
export function setLocale(tag: string): void {
  locale.messages = (tag === 'auto' ? catalogs[pickLocale()] : catalogs[tag]) ?? en;
  document.documentElement.lang = tag === 'auto' ? pickLocale() : tag;
}
