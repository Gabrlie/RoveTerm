<script lang="ts">
  import { onDestroy } from 'svelte';
  import { t } from '../i18n';
  import Icon from '../ui/Icon.svelte';
  import type { IconName } from '../ui/icons';
  import { keyBarItemLabel } from './KeyBarCustomizeDialog.svelte';
  import { builtinKeyMeta, encodeKeyItem, type KeyBarItem, type ModifierState } from './keybar';
  import { keybarPrefs } from './keybarPrefs.svelte';

  interface Props {
    ctrl: ModifierState;
    alt: ModifierState;
    keyboardOpen?: boolean;
    compactDock?: boolean;
    onctrl: () => void;
    onalt: () => void;
    /** The sequence is already encoded. Consume the armed modifiers when requested. */
    onsend: (data: string, consume?: boolean) => void;
    onpaste: () => void;
    onsnippets?: () => void;
    ontogglekeyboard: () => void;
    oncustomize?: () => void;
  }

  let {
    ctrl, alt, keyboardOpen = false, compactDock = false,
    onctrl, onalt, onsend, onpaste, onsnippets, ontogglekeyboard,
  }: Props = $props();
  const { primary, secondary } = $derived(keybarPrefs.layout);

  const actionIcons: Partial<Record<Extract<KeyBarItem, { kind: 'builtin' }>['id'], IconName>> = {
    paste: 'paste',
    snippets: 'snippets',
    keyboard: 'keyboard',
  };

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
        case 'keyboard': return ontogglekeyboard();
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
  {@const icon = id && actionIcons[id]}
  {@const label = keyBarItemLabel(item)}
  <button
    type="button"
    class="keycap"
    class:armed={modifier === 'armed'}
    class:active={id === 'keyboard' && keyboardOpen}
    title={label}
    aria-label={label}
    aria-pressed={modifier ? modifier === 'armed' : id === 'keyboard' ? keyboardOpen : undefined}
    disabled={id === 'snippets' && !onsnippets}
    onpointerdown={(event) => keyDown(event, item)}
    onpointerup={keyUp}
    onpointerleave={keyUp}
    onpointercancel={keyUp}
    onclick={(event) => clickKey(event, item)}
  >
    {#if icon}<Icon name={icon} size={16} />{:else}<span class="key-label">{label}</span>{/if}
  </button>
{/snippet}

{#snippet keyRow(items: KeyBarItem[])}
  <div class="key-row" style:--cols={Math.max(1, items.length)}>
    {#each items as item}
      {@render keycap(item)}
    {/each}
  </div>
{/snippet}

<div class="keybar" class:immersion={compactDock} role="group" aria-label={t('keybar.title')}>
  {@render keyRow(primary)}
  {#if secondary.length}
    {@render keyRow(secondary)}
  {/if}
</div>

<style>
  .keybar {
    --keycap-bg: var(--fill);
    --keycap-pressed: var(--fill-pressed);
    flex: none;
    min-width: 0;
    color: var(--fg);
    background: var(--material-bar);
    box-shadow: inset 0 1px 0 var(--separator);
  }
  .keybar.immersion {
    --keycap-bg: color-mix(in srgb, var(--term-fg) 10%, var(--term-bg));
    --keycap-pressed: color-mix(in srgb, var(--term-fg) 18%, var(--term-bg));
    --keybar-separator: color-mix(in srgb, var(--term-fg) 10%, transparent);
    background: var(--term-bg);
    color: var(--term-fg);
    box-shadow: inset 0 1px 0 var(--keybar-separator);
  }
  .key-row {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    align-items: center;
    gap: 4px;
    height: 36px;
    padding-inline: var(--space-2);
    overflow: hidden;
  }
  .keycap {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 0;
    height: 30px;
    padding: 0 2px;
    border: 1px solid transparent;
    border-radius: 7px;
    background: var(--keycap-bg);
    color: inherit;
    font: 600 11.5px var(--font-mono);
    white-space: nowrap;
    cursor: pointer;
    -webkit-touch-callout: none;
  }
  .key-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .keycap:active:not(:disabled) {
    background: var(--keycap-pressed);
  }
  .keycap.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .keycap.armed::after {
    content: '';
    position: absolute;
    left: 22%;
    right: 22%;
    bottom: 3px;
    height: 2px;
    border-radius: 1px;
    background: var(--accent);
    pointer-events: none;
  }
  .keycap:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
