<script lang="ts" module>
  import { t } from '../i18n';
  import { builtinKeyMeta, type KeyBarItem } from './keybar';

  export function keyBarItemLabel(item: KeyBarItem): string {
    if (item.kind === 'custom') return item.label;
    switch (item.id) {
      case 'paste': return t('terminal.menu.paste');
      case 'snippets': return t('action.panel.snippets');
      case 'toggleExtra': return t('host.more');
      case 'customize': return t('action.settings.open');
      default: return builtinKeyMeta(item.id).label;
    }
  }
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import Dialog from '../ui/Dialog.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import {
    addKeyBarItem,
    BUILTIN_KEY_CATALOG,
    MAX_KEYBAR_ROW_ITEMS,
    moveKeyBarItem,
    moveKeyBarItemAcrossRows,
    parseKeyBarLayout,
    removeKeyBarItem,
    type BuiltinKeyMeta,
    type KeyBarLayout,
    type KeyBarRow,
  } from './keybar';
  import { keybarPrefs } from './keybarPrefs.svelte';

  let { onclose }: { onclose: () => void } = $props();

  const rowOrder: readonly KeyBarRow[] = ['primary', 'secondary'];
  const rows = { primary: 'Primary', secondary: 'Secondary' } as const;
  const categories: Record<BuiltinKeyMeta['category'], string> = {
    modifier: 'Modifiers',
    nav: 'Navigation',
    combo: 'Shortcuts',
    symbol: 'Symbols',
    function: 'Function keys',
    action: 'Actions',
  };
  const catalog = (Object.keys(categories) as BuiltinKeyMeta['category'][]).map((category) => ({
    category,
    keys: BUILTIN_KEY_CATALOG.filter((key) => key.category === category),
  }));

  let draft = $state.raw<KeyBarLayout>(untrack(() => parseKeyBarLayout(keybarPrefs.layout)));
  let row = $state<KeyBarRow>('primary');
  let label = $state('');
  let sequence = $state('');
  const otherRow = $derived(row === 'primary' ? 'secondary' : 'primary');
  const items = $derived(draft[row]);
  const full = $derived(items.length >= MAX_KEYBAR_ROW_ITEMS);
  const lockItem = $derived(row === 'primary' && items.length === 1);
  const canMoveAcross = $derived(!lockItem && draft[otherRow].length < MAX_KEYBAR_ROW_ITEMS);

  function addCustom(event: SubmitEvent): void {
    event.preventDefault();
    const name = label.trim();
    if (!name || !sequence || full) return;
    draft = addKeyBarItem(draft, row, { kind: 'custom', id: crypto.randomUUID(), label: name, sequence });
    label = '';
    sequence = '';
  }

  function save(): void {
    keybarPrefs.save(draft);
    onclose();
  }
</script>

<Dialog title={t('action.settings.open')} width={560} {onclose}>
  <div class="row-picker" role="group" aria-label={t('action.settings.open')}>
    {#each rowOrder as key}
      <button
        type="button"
        class="secondary"
        class:selected={row === key}
        aria-pressed={row === key}
        onclick={() => (row = key)}
      >{rows[key]}</button>
    {/each}
  </div>

  <ol class="items" aria-label={rows[row]}>
    {#each items as item, index}
      <li>
        <span class="key-label" title={item.kind === 'custom' ? item.sequence : undefined}>{keyBarItemLabel(item)}</span>
        <div class="item-actions">
          <IconButton icon="back" label={t('tab.moveLeft')} disabled={index === 0} onclick={() => (draft = moveKeyBarItem(draft, row, index, -1))} />
          <IconButton icon="chevron" label={t('tab.moveRight')} disabled={index === items.length - 1} onclick={() => (draft = moveKeyBarItem(draft, row, index, 1))} />
          <IconButton icon={row === 'primary' ? 'up' : 'chevronDown'} label={'Move to ' + rows[otherRow]} disabled={!canMoveAcross} onclick={() => (draft = moveKeyBarItemAcrossRows(draft, row, index))} />
          <IconButton icon="remove" label={t('common.delete')} danger disabled={lockItem} onclick={() => (draft = removeKeyBarItem(draft, row, index))} />
        </div>
      </li>
    {/each}
  </ol>

  <div class="catalog">
    {#each catalog as { category, keys } (category)}
      <section>
        <h3>{categories[category]}</h3>
        <div class="chips">
          {#each keys as { id } (id)}
            {@const item = { kind: 'builtin', id } as const}
            <button type="button" class="secondary" disabled={full} onclick={() => (draft = addKeyBarItem(draft, row, item))}>
              {keyBarItemLabel(item)}
            </button>
          {/each}
        </div>
      </section>
    {/each}
  </div>

  <form class="custom" onsubmit={addCustom}>
    <label>
      {t('forward.label')}
      <input bind:value={label} autocomplete="off" />
    </label>
    <label>
      Sequence
      <input bind:value={sequence} placeholder={'\\x1b[A / ^C / sudo '} autocomplete="off" autocapitalize="off" spellcheck="false" />
    </label>
    <button class="secondary" type="submit" disabled={full || !label.trim() || !sequence}>{t('ai.add')}</button>
  </form>

  {#snippet actions()}
    <button class="plain reset" type="button" onclick={() => (draft = parseKeyBarLayout(null))}>{t('settings.reset')}</button>
    <button class="secondary" type="button" onclick={onclose}>{t('common.cancel')}</button>
    <button class="primary" type="button" onclick={save}>{t('common.save')}</button>
  {/snippet}
</Dialog>

<style>
  .row-picker {
    display: flex;
    gap: var(--space-2);
  }
  .row-picker > button {
    flex: 1;
  }
  .row-picker > .selected {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .items {
    display: grid;
    gap: var(--space-1);
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .items li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-inline: var(--space-3) var(--space-1);
    gap: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
  }
  .key-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--text-body) var(--font-mono);
  }
  .item-actions {
    display: flex;
    flex: none;
  }
  .item-actions > :global(button) {
    width: 40px;
    height: 40px;
  }
  .catalog {
    display: grid;
    gap: var(--space-3);
  }
  .catalog h3 {
    margin: 0 0 var(--space-1);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    font-weight: 600;
  }
  .chips, .custom {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .chips > button {
    min-width: 40px;
  }
  .custom {
    align-items: end;
  }
  .custom label {
    flex: 1 1 120px;
  }
  .custom input {
    width: 100%;
    min-width: 0;
    font-family: var(--font-mono);
  }
  .reset {
    margin-inline-end: auto;
  }
</style>
