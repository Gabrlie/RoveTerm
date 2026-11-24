<script lang="ts">
  import { onDestroy } from 'svelte';
  import { t } from '../i18n';
  import Icon from '../ui/Icon.svelte';
  import type { IconName } from '../ui/icons';
  import KeyBarCustomizeDialog, { keyBarItemLabel } from './KeyBarCustomizeDialog.svelte';
  import { builtinKeyMeta, encodeKeyItem, type KeyBarItem, type ModifierState } from './keybar';
  import { keybarPrefs } from './keybarPrefs.svelte';

  interface Props {
    ctrl: ModifierState;
    alt: ModifierState;
    keyboardOpen?: boolean;
    onctrl: () => void;
    onalt: () => void;
    /** The sequence is already encoded. Consume the armed modifiers when requested. */
    onsend: (data: string, consume?: boolean) => void;
    onpaste: () => void;
    onsnippets?: () => void;
    ontogglekeyboard: () => void;
  }

  let { ctrl, alt, keyboardOpen = false, onctrl, onalt, onsend, onpaste, onsnippets, ontogglekeyboard }: Props = $props();
  let customizing = $state(false);
  const { primary, secondary, expanded } = $derived(keybarPrefs.layout);
  const hasCustomize = $derived(
    [...primary, ...secondary].some((item) => item.kind === 'builtin' && item.id === 'customize'),
  );
  const hasPrimaryToggle = $derived(primary.some((item) => item.kind === 'builtin' && item.id === 'toggleExtra'));

  const actionIcons: Partial<Record<Extract<KeyBarItem, { kind: 'builtin' }>['id'], IconName>> = {
    paste: 'paste',
    snippets: 'snippets',
    keyboard: 'keyboard',
    customize: 'settings',
  };
  const extraKey = { kind: 'builtin', id: 'toggleExtra' } as const;
  const settingsKey = { kind: 'builtin', id: 'customize' } as const;

  const REPEAT_DELAY_MS = 380;
  const REPEAT_INTERVAL_MS = 65;
  let repeatTimer = 0;
  let repeatInterval = 0;
  let repeatPointer: number | undefined;
  let repeatedButton: EventTarget | null = null;

  function press(item: KeyBarItem): void {
    if (item.kind === 'builtin') {
      switch (item.id) {
        case 'ctrl': return onctrl();
        case 'alt': return onalt();
        case 'paste': return onpaste();
        case 'snippets': return onsnippets?.();
        case 'toggleExtra': return keybarPrefs.setExpanded(!expanded);
        case 'keyboard': return ontogglekeyboard();
        case 'customize': customizing = true; return;
      }
    }
    const next = encodeKeyItem(item, ctrl, alt);
    if (next?.data) onsend(next.data, next.ctrl !== ctrl || next.alt !== alt);
  }

  function stopRepeat(): void {
    clearTimeout(repeatTimer);
    clearInterval(repeatInterval);
    repeatPointer = undefined;
  }

  function keyDown(event: PointerEvent, item: KeyBarItem): void {
    event.preventDefault();
    if (!event.isPrimary || event.button !== 0) return;
    stopRepeat();
    repeatedButton = null;
    if (item.kind !== 'builtin' || !builtinKeyMeta(item.id).repeatable) return;
    const button = event.currentTarget as HTMLButtonElement;
    if (button.hasPointerCapture(event.pointerId)) button.releasePointerCapture(event.pointerId);
    repeatPointer = event.pointerId;
    const repeat = () => {
      if (!button.isConnected) return stopRepeat();
      repeatedButton = button;
      press(item);
    };
    repeatTimer = window.setTimeout(() => {
      repeatInterval = window.setInterval(repeat, REPEAT_INTERVAL_MS);
      repeat();
    }, REPEAT_DELAY_MS);
  }

  function keyUp(event: PointerEvent): void {
    if (repeatPointer === event.pointerId) stopRepeat();
  }

  function clickKey(event: MouseEvent, item: KeyBarItem): void {
    if (event.detail > 0 && repeatedButton === event.currentTarget) {
      repeatedButton = null;
      return;
    }
    press(item);
  }

  onDestroy(stopRepeat);
</script>

{#snippet keycap(item: KeyBarItem)}
  {@const id = item.kind === 'builtin' ? item.id : undefined}
  {@const modifier = id === 'ctrl' ? ctrl : id === 'alt' ? alt : undefined}
  {@const icon = id === 'toggleExtra' ? (expanded ? 'up' : 'more') : id && actionIcons[id]}
  {@const label = keyBarItemLabel(item)}
  {@const accessibleLabel = modifier === 'locked' ? t(id === 'ctrl' ? 'keybar.ctrlLocked' : 'keybar.altLocked') : label}
  <button
    type="button"
    class="keycap"
    class:armed={modifier === 'armed'}
    class:locked={modifier === 'locked'}
    class:active={(id === 'keyboard' && keyboardOpen) || (id === 'toggleExtra' && expanded)}
    title={accessibleLabel}
    aria-label={accessibleLabel}
    aria-pressed={modifier ? (modifier === 'armed' ? 'mixed' : modifier === 'locked') : id === 'keyboard' ? keyboardOpen : undefined}
    aria-expanded={id === 'toggleExtra' ? expanded : undefined}
    aria-haspopup={id === 'customize' ? 'dialog' : undefined}
    disabled={id === 'snippets' && !onsnippets}
    onpointerdown={(event) => keyDown(event, item)}
    onpointerup={keyUp}
    onpointerleave={keyUp}
    onpointercancel={keyUp}
    onclick={(event) => clickKey(event, item)}
  >
    {#if icon}<Icon name={icon} size={16} />{:else}{label}{/if}
  </button>
{/snippet}

<div class="keybar" role="group" aria-label={t('keybar.title')}>
  {#if expanded}
    <div class="key-row">
      {#each secondary as item}
        {@render keycap(item)}
      {/each}
      {#if !hasCustomize}
        {@render keycap(settingsKey)}
      {/if}
    </div>
  {/if}
  <div class="key-row">
    {#each primary as item}
      {@render keycap(item)}
    {/each}
    {#if !hasPrimaryToggle}
      {@render keycap(extraKey)}
    {/if}
  </div>
</div>

{#if customizing}
  <KeyBarCustomizeDialog onclose={() => (customizing = false)} />
{/if}

<style>
  .keybar {
    flex: none;
    min-width: 0;
    color: var(--fg);
    background: var(--material-bar);
    box-shadow: inset 0 1px 0 var(--separator);
  }
  .key-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding-inline: var(--space-2);
    height: 40px;
    overflow-x: auto;
    touch-action: pan-x;
    scrollbar-width: none;
  }
  .keycap {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    min-width: var(--space-8);
    height: var(--space-8);
    padding: 0 var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: inherit;
    font: 600 var(--text-body) var(--font-mono);
    white-space: nowrap;
    cursor: pointer;
    -webkit-touch-callout: none;
  }
  .keycap:active:not(:disabled) {
    background: var(--fill-pressed);
  }
  .keycap.armed, .keycap.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .keycap.armed {
    border-color: var(--accent);
  }
  .keycap.armed::after {
    content: '';
    width: var(--space-1);
    height: var(--space-1);
    border-radius: var(--radius-pill);
    background: currentColor;
  }
  .keycap.locked {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .keycap:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
