<script lang="ts">
  // Every icon-only action uses this button. `label` is both the tooltip and the accessible name.
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';

  interface Props {
    icon: IconName;
    label: string;
    onclick: (event: MouseEvent) => void;
    active?: boolean;
    danger?: boolean;
    disabled?: boolean;
    small?: boolean;
  }

  let { icon, label, onclick, active = false, danger = false, disabled = false, small = false }: Props = $props();
</script>

<button
  type="button"
  class="icon-button"
  class:active
  class:danger
  class:small
  title={label}
  aria-label={label}
  aria-pressed={active}
  {disabled}
  {onclick}
>
  <Icon name={icon} size={small ? 14 : 16} />
</button>

<style>
  .icon-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height);
    height: var(--control-height);
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }
  .icon-button.small {
    width: 24px;
    height: 24px;
  }
  .icon-button:hover:not(:disabled) {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .icon-button:active:not(:disabled) {
    background: var(--fill-pressed);
  }
  .icon-button.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .icon-button.danger:hover:not(:disabled) {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .icon-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .icon-button:disabled {
    opacity: 0.35;
    cursor: default;
  }
</style>
