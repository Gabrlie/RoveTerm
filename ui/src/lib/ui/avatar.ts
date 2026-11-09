// Identity marks for hosts: a stable system colour and one or two letters from the name.

const TINTS = ['blue', 'purple', 'pink', 'red', 'orange', 'yellow', 'green', 'teal'] as const;

/** A `--sys-*` colour chosen from the name; the same name always gets the same colour. */
export function avatarColor(name: string): string {
  let hash = 0;
  for (const char of name) hash = (hash * 31 + (char.codePointAt(0) ?? 0)) >>> 0;
  return `var(--sys-${TINTS[hash % TINTS.length]})`;
}

/** First letter of the first two words, or the first two characters of a single word. */
export function initials(name: string): string {
  const [first, second] = name.trim().split(/[\s._-]+/).filter(Boolean);
  if (!first) return '?';
  const letters = second ? [...first].slice(0, 1).join('') + [...second].slice(0, 1).join('') : [...first].slice(0, 2).join('');
  return letters.toUpperCase();
}
