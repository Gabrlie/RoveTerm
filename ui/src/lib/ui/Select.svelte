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
  <span class="chevron"><Icon name="chevronsUpDown" size={14} /></span>
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
  /* Trigger looks like a macOS pop-up button; the list is a glass menu. */
  .select {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    height: var(--control-height);
    padding: 0 6px 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--border),
      0 0.5px 1.5px rgb(0 0 0 / 0.08);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-body);
    text-align: left;
    cursor: pointer;
    transition: box-shadow var(--dur-fast) var(--ease-out);
  }
  .select:focus-visible,
  .select.open {
    outline: none;
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 70%, transparent),
      var(--focus-ring);
  }
  .select:disabled {
    opacity: 0.5;
    cursor: default;
  }
  /* Up-down chevrons like a system pop-up button, centred on the right. */
  .chevron {
    display: inline-flex;
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
    padding: 5px;
    overflow-y: auto;
    list-style: none;
    border-radius: var(--radius-md);
    background: var(--material-popover);
    box-shadow: var(--glass-edge), var(--shadow-lg);
    animation: pop var(--dur-fast) var(--ease-out);
  }
  .list.above {
    transform: translateY(-100%);
    animation: none;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    height: 26px;
    padding: 0 var(--space-3) 0 var(--space-1);
    border-radius: var(--radius-xs);
    color: var(--fg);
    font-size: var(--text-body);
    white-space: nowrap;
    cursor: default;
  }
  li.highlighted {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .mark {
    display: inline-flex;
    width: 16px;
    color: inherit;
  }
</style>
