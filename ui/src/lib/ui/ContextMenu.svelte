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
  // A menu at a point, for right-click and "more" buttons. Moved to <body> so no container clips
  // it; closes on selection, Escape, a click elsewhere, or window resize.
  import { onMount } from 'svelte';
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
    switch (event.key) {
      case 'ArrowDown':
        highlighted = (highlighted + 1) % items.length;
        break;
      case 'ArrowUp':
        highlighted = (highlighted - 1 + items.length) % items.length;
        break;
      case 'Enter':
      case ' ':
        choose(items[highlighted]);
        break;
      case 'Escape':
        onclose();
        break;
      case 'ArrowLeft':
      case 'ArrowRight':
        if (!onstep) return;
        onstep(event.key === 'ArrowLeft' ? -1 : 1);
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  function onPointerDown(event: PointerEvent): void {
    const target = event.target as Node;
    if (!menu?.contains(target) && !owner?.contains(target)) onclose();
  }

  onMount(() => {
    // Keep the menu inside the window.
    const rect = menu?.getBoundingClientRect();
    const width = rect?.width ?? 0;
    const height = rect?.height ?? 0;
    position = {
      left: Math.max(4, Math.min(x, window.innerWidth - width - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - height - 4)),
    };
    menu?.focus();
  });
</script>

<svelte:window onpointerdowncapture={onPointerDown} onresize={onclose} />

<div
  use:portal
  bind:this={menu}
  class="menu"
  role="menu"
  tabindex="-1"
  style:left="{position.left}px"
  style:top="{position.top}px"
  onkeydown={onKeydown}
>
  {#each items as item, index (item.label)}
    <button
      type="button"
      role="menuitem"
      class:separated={item.separated}
      class:danger={item.danger}
      class:highlighted={index === highlighted}
      onpointerenter={() => (highlighted = index)}
      onclick={() => choose(item)}
    >
      <span class="icon">{#if item.icon}<Icon name={item.icon} size={14} />{/if}</span>
      <span class="label">{item.label}</span>
      {#if item.hint}<span class="hint">{item.hint}</span>{/if}
    </button>
  {/each}
</div>

<style>
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
