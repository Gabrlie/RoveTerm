/** Merges comma- or newline-delimited rules into the current list. */
export function appendRules(rules: readonly string[], text: string): string[] {
  const added = text.split(/[,\r\n]+/).map((rule) => rule.trim()).filter(Boolean);
  return [...new Set([...rules, ...added])];
}

export function removeRule(rules: readonly string[], rule: string): string[] {
  return rules.filter((entry) => entry !== rule);
}
