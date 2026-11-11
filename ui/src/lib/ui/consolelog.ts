// Warnings and errors of the interface also go to the application log (the files a user sends
// along with a problem). Only `console.warn` / `console.error` and uncaught errors are taken.

/** Longest message sent; the backend cuts at the same length. */
const MAX = 4096;

/** The text of a console call's arguments: strings as they are, errors with their message and
 *  the first lines of their stack, other values as JSON. */
export function consoleText(args: readonly unknown[]): string {
  const text = args
    .map((arg) => {
      if (typeof arg === 'string') return arg;
      if (arg instanceof Error) return `${arg.name}: ${arg.message}${arg.stack ? `\n${arg.stack.split('\n').slice(1, 6).join('\n')}` : ''}`;
      try {
        return JSON.stringify(arg) ?? String(arg);
      } catch {
        return String(arg);
      }
    })
    .join(' ');
  return text.length > MAX ? `${text.slice(0, MAX - 1)}…` : text;
}

/** Sends warnings and errors to `write` as well as to the console; a failed send is not logged
 *  again. */
export function forwardConsole(write: (level: 'warn' | 'error', message: string) => Promise<void>): void {
  for (const level of ['warn', 'error'] as const) {
    const original = console[level].bind(console);
    console[level] = (...args: unknown[]) => {
      original(...args);
      write(level, consoleText(args)).catch(() => undefined);
    };
  }
  window.addEventListener('error', (event) => {
    write('error', consoleText([event.error ?? event.message])).catch(() => undefined);
  });
  window.addEventListener('unhandledrejection', (event) => {
    write('error', consoleText(['unhandled rejection', event.reason])).catch(() => undefined);
  });
}
