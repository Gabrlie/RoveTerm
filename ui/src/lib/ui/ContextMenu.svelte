<script lang="ts" module>
  import type { IconName } from './icons';

  export interface MenuItem {
    label: string;
    icon?: IconName;
    danger?: boolean;
    /** Draws a separator above the item. */
    separated?: boolean;
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
  }

  let { x, y, items, onclose }: Props = $props();
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
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  function onPointerDown(event: PointerEvent): void {
    if (!menu?.contains(event.target as Node)) onclose();
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
      {item.label}
    </button>
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 1000;
    min-width: 180px;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    box-shadow: 0 8px 24px rgb(0 0 0 / 18%);
    outline: none;
  }
  button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    height: 30px;
    padding: 0 var(--space-3) 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-body);
    text-align: left;
    cursor: pointer;
  }
  button.highlighted {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  button.danger {
    color: var(--danger);
  }
  button.separated {
    margin-top: var(--space-1);
    position: relative;
  }
  button.separated::before {
    content: '';
    position: absolute;
    top: calc(-1 * var(--space-1) / 2 - 1px);
    left: var(--space-2);
    right: var(--space-2);
    border-top: 1px solid var(--border);
  }
  .icon {
    display: inline-flex;
    width: 16px;
    color: var(--fg-muted);
  }
  button.danger .icon {
    color: var(--danger);
  }
</style>
