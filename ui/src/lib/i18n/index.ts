// Locale selection and message formatting. UI copy always goes through `t`.
import { en, type MessageKey, type Messages } from './en';
import { zhCN } from './zh-CN';
import { isCommandError } from '../ipc';

const catalogs: Record<string, Messages> = { en, 'zh-CN': zhCN };

function pickLocale(): string {
  const preferred = navigator.languages ?? [navigator.language];
  for (const tag of preferred) {
    if (tag.toLowerCase().startsWith('zh')) return 'zh-CN';
    if (tag.toLowerCase().startsWith('en')) return 'en';
  }
  return 'en';
}

let messages: Messages = catalogs[pickLocale()] ?? en;

export type { MessageKey };

/** Formats `key`, replacing `{name}` placeholders with `params`. */
export function t(key: MessageKey, params: Record<string, string | number> = {}): string {
  const template = messages[key] ?? en[key];
  return template.replace(/\{(\w+)\}/g, (_, name: string) => String(params[name] ?? `{${name}}`));
}

/** Localizes a command error by its code, falling back to the generic message. */
/** Codes whose backend message names the cause (a path, an OS error) and is shown after the text. */
const DETAILED = new Set(['ssh.key', 'ssh.agent', 'ssh.connect', 'ssh.protocol', 'ssh.sftp', 'ssh.proxy', 'ssh.jump', 'ssh.jump_host', 'fs.io']);

export function errorText(error: unknown): string {
  if (isCommandError(error)) {
    const key = `error.${error.code}` as MessageKey;
    if (!(key in en)) return `${t('error.unknown')}: ${error.message}`;
    return DETAILED.has(error.code) ? `${t(key)}: ${error.message}` : t(key);
  }
  return `${t('error.unknown')}: ${String(error)}`;
}

/** Switches the active locale for subsequently rendered text. */
export function setLocale(locale: string): void {
  messages = catalogs[locale] ?? en;
}
