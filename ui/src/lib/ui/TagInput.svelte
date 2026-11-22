<script lang="ts">
  import { t } from '../i18n';
  import { appendRules, removeRule } from '../settings/sections/tagRules';

  type RuleEdit = (rules: readonly string[]) => string[];
  interface Props {
    label: string;
    rules: readonly string[];
    onchange: (edit: RuleEdit) => Promise<void>;
  }

  let { label, rules, onchange }: Props = $props();
  let input = $state<HTMLInputElement>();
  let text = $state('');
  let pending = $state(0);
  let preview = $state<string[]>([]);
  const tags = $derived(pending ? preview : [...new Set(rules)]);

  async function edit(change: RuleEdit): Promise<void> {
    preview = change(tags);
    pending += 1;
    try {
      await onchange(change);
    } finally {
      pending -= 1;
    }
  }

  function add(value: string): void {
    text = '';
    if (appendRules(tags, value).length > tags.length) void edit((entries) => appendRules(entries, value));
  }

  function remove(rule: string): void {
    void edit((entries) => removeRule(entries, rule));
    input?.focus();
  }

  function onkeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    const last = tags.at(-1);
    if (event.key === 'Enter' || event.key === ',') {
      event.preventDefault();
      add(text);
    } else if (event.key === 'Backspace' && !text && last !== undefined) {
      event.preventDefault();
      remove(last);
    }
  }

  function onpaste(event: ClipboardEvent): void {
    const pasted = event.clipboardData?.getData('text') ?? '';
    if (!/[,\r\n]/.test(pasted)) return;
    event.preventDefault();
    const from = input?.selectionStart ?? text.length;
    const to = input?.selectionEnd ?? from;
    add(text.slice(0, from) + pasted + text.slice(to));
  }
</script>

<div class="tags" role="group" aria-label={label}>
  {#each tags as tag (tag)}
    <span class="tag">
      {tag}
      <button type="button" class="plain" aria-label={t('ai.removeRule', { rule: tag })} onclick={() => remove(tag)}>×</button>
    </span>
  {/each}
  <input class="bare" bind:this={input} bind:value={text} aria-label={label} placeholder={t('ai.addRule')} spellcheck="false" {onkeydown} {onpaste} />
</div>

<style>
  .tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
    min-height: var(--control-height);
    padding: var(--space-1);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    background: var(--surface);
    font: var(--text-footnote) var(--font-mono);
  }
  .tags:focus-within { box-shadow: var(--focus-ring); }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    max-width: 100%;
    padding-left: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg);
    overflow-wrap: anywhere;
  }
  .tag button { height: 22px; padding: 0 var(--space-1); color: var(--fg-muted); }
  .tag button:hover { color: var(--danger); }
  input { flex: 1 1 10em; min-width: 0; padding: var(--space-1); font: inherit; }
</style>
