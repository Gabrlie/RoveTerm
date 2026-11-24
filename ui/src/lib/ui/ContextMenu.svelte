<script lang="ts" module>
  import type { IconName } from './icons';

  export interface MenuItem {
    label: string;
    icon?: IconName;
    danger?: boolean;
    /** Draws a separator above the item. */
    separated?: boolean;
    /** Right-aligned text, such as the shortcut. */
    hint?: string;
    onselect: () => void;
  }
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../i18n';
  import { viewport } from '../layout/viewport.svelte';
  import { handleBack, pushBackHandler } from './backStack';
  import Icon from './Icon.svelte';

  interface Props {
    x: number;
    y: number;
    items: MenuItem[];
    onclose: () => void;
    /** Clicks inside this element do not close the menu (the menu bar handles them itself). */
    owner?: HTMLElement;
    /** Left and right arrows, for moving between the menus of a menu bar. */
    onstep?: (direction: -1 | 1) => void;
  }

  let { x, y, items, onclose, owner, onstep }: Props = $props();
  let menu = $state<HTMLDivElement>();
  let highlighted = $state(0);
  let position = $state({ left: 0, top: 0 });

  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  function choose(item: MenuItem | undefined): void {
    onclose();
    item?.onselect();
  }

  function onKeydown(event: KeyboardEvent): void {
    const itemCount = items.length + (viewport.compact ? 1 : 0);
    switch (event.key) {
      case 'ArrowDown':
        if (itemCount) highlighted = (highlighted + 1) % itemCount;
        break;
      case 'ArrowUp':
        if (itemCount) highlighted = (highlighted - 1 + itemCount) % itemCount;
        break;
      case 'Enter':
      case ' ':
        if (viewport.compact && event.target instanceof HTMLButtonElement) return;
        choose(items[highlighted]);
        break;
      case 'Escape':
        handleBack();
        break;
      case 'ArrowLeft':
      case 'ArrowRight':
        if (!onstep) return;
        onstep(event.key === 'ArrowLeft' ? -1 : 1);
        break;
      default:
        return;
    }
    if (viewport.compact && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')[highlighted]?.focus();
    }
    event.preventDefault();
    event.stopPropagation();
  }

  function onPointerDown(event: PointerEvent): void {
    if (viewport.compact) return;
    const target = event.target as Node;
    if (!menu?.contains(target) && !owner?.contains(target)) onclose();
  }

  function onBackdropPointerDown(event: PointerEvent): void {
    if (viewport.compact && event.target === event.currentTarget) onclose();
  }

  function onResize(): void {
    if (!viewport.compact) onclose();
  }

  onMount(() => {
    const disposeBack = pushBackHandler(() => onclose());
    // Keep the menu inside the window.
    const rect = menu?.getBoundingClientRect();
    const width = rect?.width ?? 0;
    const height = rect?.height ?? 0;
    position = {
      left: Math.max(4, Math.min(x, window.innerWidth - width - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - height - 4)),
    };
    menu?.focus();
    return disposeBack;
  });
</script>

<svelte:window onpointerdowncapture={onPointerDown} onresize={onResize} />

<div class="menu-layer" class:compact={viewport.compact} role="presentation" use:portal onpointerdown={onBackdropPointerDown}>
  <div
    bind:this={menu}
    class="menu"
    role="menu"
    tabindex="-1"
    style:left={viewport.compact ? undefined : `${position.left}px`}
    style:top={viewport.compact ? undefined : `${position.top}px`}
    onkeydown={onKeydown}
  >
    {#if viewport.compact}
      <div class="grab-handle" aria-hidden="true"></div>
    {/if}
    <div class="items" role="presentation">
      {#each items as item, index (item.label)}
        <button
          type="button"
          role="menuitem"
          class:separated={item.separated}
          class:danger={item.danger}
          class:highlighted={index === highlighted}
          onfocus={() => { if (viewport.compact) highlighted = index; }}
          onpointerenter={() => (highlighted = index)}
          onclick={() => choose(item)}
        >
          <span class="icon">{#if item.icon}<Icon name={item.icon} size={viewport.compact ? 16 : 14} />{/if}</span>
          <span class="label">{item.label}</span>
          {#if item.hint}<span class="hint">{item.hint}</span>{/if}
        </button>
      {/each}
    </div>
    {#if viewport.compact}
      <button
        type="button"
        role="menuitem"
        class="separated cancel"
        class:highlighted={highlighted === items.length}
        onfocus={() => (highlighted = items.length)}
        onpointerenter={() => (highlighted = items.length)}
        onclick={onclose}
      >
        {t('common.cancel')}
      </button>
    {/if}
  </div>
</div>

<style>
  .menu-layer,
  .items {
    display: contents;
  }
  .menu-layer.compact {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: flex-end;
    background: var(--overlay);
  }
  /* A glass menu: accent-filled highlight, hairline separators. */
  .menu {
    position: fixed;
    z-index: 1000;
    min-width: 200px;
    padding: 5px;
    border-radius: var(--radius-md);
    background: var(--material-popover);
    box-shadow: var(--glass-edge), var(--shadow-lg);
    outline: none;
    animation: pop var(--dur-fast) var(--ease-out);
  }
  .compact .menu {
    position: static;
    display: flex;
    flex-direction: column;
    width: 100%;
    min-width: 0;
    max-height: 100dvh;
    padding: var(--space-2) max(var(--space-3), var(--safe-right))
      max(var(--space-3), var(--safe-bottom)) max(var(--space-3), var(--safe-left));
    border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  }
  .grab-handle {
    flex: none;
    width: var(--space-8);
    height: var(--space-1);
    margin: 0 auto var(--space-2);
    border-radius: var(--radius-pill);
    background: var(--separator);
  }
  .compact .items {
    display: block;
    max-height: 70dvh;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.97);
    }
  }
  button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    height: 26px;
    padding: 0 var(--space-3) 0 var(--space-2);
    border: none;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-body);
    text-align: left;
    cursor: default;
  }
  .compact button {
    min-height: var(--touch-target, 44px);
    height: auto;
    padding: var(--space-2) var(--space-3);
  }
  .compact .icon {
    flex: 0 0 16px;
  }
  .cancel {
    flex: none;
    justify-content: center;
  }
  button.highlighted {
    background: var(--accent);
    color: var(--accent-fg);
  }
  button.danger {
    color: var(--danger);
  }
  button.danger.highlighted {
    background: var(--danger);
    color: #fff;
  }
  button.separated {
    margin-top: 9px;
    position: relative;
  }
  button.separated::before {
    content: '';
    position: absolute;
    top: -5px;
    left: var(--space-2);
    right: var(--space-2);
    height: 1px;
    background: var(--separator);
  }
  .label {
    flex: 1;
  }
  .hint {
    margin-left: var(--space-6);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  button.highlighted .hint {
    color: inherit;
    opacity: 0.85;
  }
  .icon {
    display: inline-flex;
    width: 16px;
    color: var(--fg-muted);
  }
  button.highlighted .icon,
  button.danger .icon {
    color: inherit;
  }
</style>
