// Scheduled tasks as the dialog shows and edits them.
import type { Repeat } from '../ipc';

/** A new task's id from its name: a slug, made unique among `taken`. */
export function scheduleId(name: string, taken: readonly string[]): string {
  const base =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 30) || 'task';
  let id = base;
  for (let n = 2; taken.includes(id); n += 1) id = `${base}-${n}`;
  return id;
}

const pad = (n: number): string => String(n).padStart(2, '0');

/** `09:05`. */
export function clock(hour: number, minute: number): string {
  return `${pad(hour)}:${pad(minute)}`;
}

/** Hour and minute of `HH:MM`, or null. */
export function parseClock(text: string): { hour: number; minute: number } | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(text.trim());
  if (!match) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  return hour < 24 && minute < 60 ? { hour, minute } : null;
}

/** The key and values describing `repeat` for the user (weekday 0 is Monday). */
export function repeatText(repeat: Repeat): { key: 'once' | 'hourly' | 'daily' | 'weekly'; values: Record<string, string | number> } {
  switch (repeat.kind) {
    case 'once':
      return { key: 'once', values: { time: new Date(repeat.at).toLocaleString([], { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }) } };
    case 'hourly':
      return { key: 'hourly', values: { minute: pad(repeat.minute) } };
    case 'daily':
      return { key: 'daily', values: { time: clock(repeat.hour, repeat.minute) } };
    case 'weekly':
      return { key: 'weekly', values: { day: repeat.weekday, time: clock(repeat.hour, repeat.minute) } };
  }
}
