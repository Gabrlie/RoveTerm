<script lang="ts">
  import { onDestroy } from 'svelte';
  import { t } from '../i18n';
  import Icon from '../ui/Icon.svelte';
  import type { IconName } from '../ui/icons';
  import KeyBarCustomizeDialog, { keyBarItemLabel } from './KeyBarCustomizeDialog.svelte';
  import { builtinKeyMeta, encodeKeyItem, isPinnedKey, type KeyBarItem, type ModifierState } from './keybar';
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
    onctrl, onalt, onsend, onpaste, onsnippets, ontogglekeyboard, oncustomize,
  }: Props = $props();
  let customizing = $state(false);
  const { primary, secondary, expanded } = $derived(keybarPrefs.layout);
  const toggleLabel = $derived(t(expanded ? 'terminal.dock.collapseKeys' : 'terminal.dock.expandKeys'));
  const customizeLabel = $derived(t('terminal.dock.customizeKeys'));

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

  function customize(): void {
    if (oncustomize) oncustomize();
    else customizing = true;
  }

  onDestroy(stopRepeat);
</script>

{#snippet keycap(item: KeyBarItem)}
  {@const id = item.kind === 'builtin' ? item.id : undefined}
  {@const modifier = id === 'ctrl' ? ctrl : id === 'alt' ? alt : undefined}
  {@const icon = id && actionIcons[id]}
  {@const label = keyBarItemLabel(item)}
  {@const accessibleLabel = modifier === 'locked' ? t(id === 'ctrl' ? 'keybar.ctrlLocked' : 'keybar.altLocked') : label}
  <button
    type="button"
    class="keycap"
    class:armed={modifier === 'armed'}
    class:locked={modifier === 'locked'}
    class:active={id === 'keyboard' && keyboardOpen}
    title={accessibleLabel}
    aria-label={accessibleLabel}
    aria-pressed={modifier ? (modifier === 'armed' ? 'mixed' : modifier === 'locked') : id === 'keyboard' ? keyboardOpen : undefined}
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

{#snippet keyStrip(items: KeyBarItem[])}
  <div class="key-strip">
    {#each items as item}
      {#if item.kind === 'custom' || (!isPinnedKey(item.id) && !(compactDock && item.id === 'keyboard'))}
        {@render keycap(item)}
      {/if}
    {/each}
  </div>
{/snippet}

<div class="keybar" class:immersion={compactDock} role="group" aria-label={t('keybar.title')}>
  {#if expanded}
    <div class="key-row">
      {@render keyStrip(secondary)}
      <div class="pinned-actions">
        <button
          type="button"
          class="keycap"
          title={customizeLabel}
          aria-label={customizeLabel}
          aria-haspopup="dialog"
          onpointerdown={(event) => event.preventDefault()}
          onclick={customize}
        >
          <Icon name="settings" size={16} />
        </button>
      </div>
    </div>
  {/if}
  <div class="key-row">
    {@render keyStrip(primary)}
    <div class="pinned-actions">
      <button
        type="button"
        class="keycap"
        title={toggleLabel}
        aria-label={toggleLabel}
        aria-expanded={expanded}
        onpointerdown={(event) => event.preventDefault()}
        onclick={() => keybarPrefs.setExpanded(!expanded)}
      >
        <Icon name={expanded ? 'chevronDown' : 'chevronUp'} size={16} />
      </button>
    </div>
  </div>
</div>

{#if customizing}
  <KeyBarCustomizeDialog onclose={() => (customizing = false)} />
{/if}

<style>
  .keybar {
    --keycap-bg: var(--fill);
    --keycap-pressed: var(--fill-pressed);
    --keybar-separator: var(--separator);
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
    display: flex;
    align-items: center;
    height: 40px;
    padding-inline-end: var(--space-2);
  }
  .immersion .key-row {
    height: 38px;
  }
  .key-strip {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 3px 6px 3px var(--space-2);
    overflow-x: auto;
    touch-action: pan-x;
    scrollbar-width: none;
  }
  .pinned-actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding-left: 6px;
    box-shadow: inset 1px 0 0 var(--keybar-separator);
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
    background: var(--keycap-bg);
    color: inherit;
    font: 600 var(--text-body) var(--font-mono);
    white-space: nowrap;
    cursor: pointer;
    -webkit-touch-callout: none;
  }
  .immersion .keycap {
    height: 32px;
    border-radius: 7px;
  }
  .keycap:active:not(:disabled) {
    background: var(--keycap-pressed);
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
