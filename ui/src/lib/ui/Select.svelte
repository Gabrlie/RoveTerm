<script lang="ts" generics="T extends string">
  // Every drop-down uses this control instead of the native <select>, so it follows the theme
  // tokens on every platform. The list is a fixed-position popover moved to <body>: scrolling
  // containers (dialog bodies) cannot clip it, and clicks on it never reach a surrounding <label>.
  import Icon from './Icon.svelte';

  interface Option {
    value: T;
    label: string;
  }

  interface Props {
    value: T;
    options: Option[];
    onchange?: (value: T) => void;
    disabled?: boolean;
    /** Accessible name when the control is not inside a <label>. */
    label?: string;
  }

  let { value = $bindable(), options, onchange, disabled = false, label }: Props = $props();

  const listId = `select-${Math.random().toString(36).slice(2, 10)}`;
  let open = $state(false);
  let highlighted = $state(0);
  let trigger = $state<HTMLButtonElement>();
  let position = $state({ left: 0, top: 0, width: 0, above: false });

  const selected = $derived(options.find((option) => option.value === value));

  function place(): void {
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const listHeight = Math.min(options.length * 30 + 8, 260);
    const above = rect.bottom + listHeight > window.innerHeight && rect.top > listHeight;
    position = { left: rect.left, top: above ? rect.top - 4 : rect.bottom + 4, width: rect.width, above };
  }

  function show(): void {
    if (disabled || options.length === 0) return;
    place();
    highlighted = Math.max(
      0,
      options.findIndex((option) => option.value === value),
    );
    open = true;
  }

  function choose(index: number): void {
    const option = options[index];
    open = false;
    trigger?.focus();
    if (!option || option.value === value) return;
    value = option.value;
    onchange?.(option.value);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) {
        event.preventDefault();
        show();
      }
      return;
    }
    switch (event.key) {
      case 'ArrowDown':
        highlighted = (highlighted + 1) % options.length;
        break;
      case 'ArrowUp':
        highlighted = (highlighted - 1 + options.length) % options.length;
        break;
      case 'Home':
        highlighted = 0;
        break;
      case 'End':
        highlighted = options.length - 1;
        break;
      case 'Enter':
      case ' ':
        choose(highlighted);
        break;
      case 'Escape':
        // Closing the list must not also close the surrounding dialog.
        event.stopPropagation();
        open = false;
        break;
      case 'Tab':
        open = false;
        return;
      default:
        return;
    }
    event.preventDefault();
  }

  /** Moves the node to <body> for its lifetime. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  function onWindowPointerDown(event: PointerEvent): void {
    if (!open) return;
    const target = event.target as Node;
    if (trigger?.contains(target) || document.getElementById(listId)?.contains(target)) return;
    open = false;
  }
</script>

<svelte:window onpointerdowncapture={onWindowPointerDown} onresize={() => (open = false)} />

<button
  bind:this={trigger}
  type="button"
  class="select"
  class:open
  role="combobox"
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-controls={listId}
  aria-label={label}
  {disabled}
  onclick={() => {
    if (open) open = false;
    else show();
  }}
  onkeydown={onKeydown}
>
  <span class="value">{selected?.label ?? ''}</span>
  <Icon name="chevronDown" size={14} />
</button>

{#if open}
  <ul
    use:portal
    id={listId}
    class="list"
    class:above={position.above}
    role="listbox"
    style:left="{position.left}px"
    style:top="{position.top}px"
    style:min-width="{position.width}px"
  >
    {#each options as option, index (option.value)}
      <li
        role="option"
        aria-selected={option.value === value}
        class:highlighted={index === highlighted}
        onpointerenter={() => (highlighted = index)}
        onpointerdown={(event) => event.preventDefault()}
        onclick={() => choose(index)}
        onkeydown={() => {}}
      >
        <span class="mark">{#if option.value === value}<Icon name="check" size={14} />{/if}</span>
        {option.label}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .select {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    padding: 6px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-body);
    text-align: left;
    cursor: pointer;
  }
  .select:focus-visible,
  .select.open {
    outline: 2px solid color-mix(in srgb, var(--accent) 60%, transparent);
    outline-offset: -1px;
  }
  .select:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .select :global(svg) {
    flex: none;
    color: var(--fg-muted);
  }
  .value {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list {
    position: fixed;
    z-index: 1000;
    max-height: 260px;
    margin: 0;
    padding: var(--space-1);
    overflow-y: auto;
    list-style: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    box-shadow: 0 8px 24px rgb(0 0 0 / 18%);
  }
  .list.above {
    transform: translateY(-100%);
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    height: 30px;
    padding: 0 var(--space-3) 0 var(--space-1);
    border-radius: var(--radius-sm);
    color: var(--fg);
    font-size: var(--text-body);
    white-space: nowrap;
    cursor: pointer;
  }
  li.highlighted {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .mark {
    display: inline-flex;
    width: 16px;
    color: var(--accent);
  }
</style>
