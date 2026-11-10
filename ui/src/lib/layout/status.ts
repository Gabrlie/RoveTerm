// Formatting for the status bar.

/** Whole days, hours and minutes in `ms`. */
export interface Duration {
  days: number;
  hours: number;
  minutes: number;
}

export function durationParts(ms: number): Duration {
  const minutes = Math.max(0, Math.floor(ms / 60_000));
  return { days: Math.floor(minutes / 1440), hours: Math.floor((minutes % 1440) / 60), minutes: minutes % 60 };
}

/** Which message shows a duration: under a minute, minutes, hours and minutes, or days and hours. */
export function durationKind(parts: Duration): 'now' | 'minutes' | 'hours' | 'days' {
  if (parts.days > 0) return 'days';
  if (parts.hours > 0) return 'hours';
  return parts.minutes > 0 ? 'minutes' : 'now';
}

/** `HH:mm` in local time. */
export function clock(date: Date): string {
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}

/** Milliseconds until the next minute starts, to refresh the clock exactly then. */
export function untilNextMinute(date: Date): number {
  return 60_000 - (date.getSeconds() * 1000 + date.getMilliseconds());
}
