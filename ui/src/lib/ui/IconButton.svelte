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
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    cursor: pointer;
  }
  .icon-button.small {
    width: 22px;
    height: 22px;
  }
  .icon-button:hover:not(:disabled) {
    background: var(--bg-sunken);
    color: var(--fg);
  }
  .icon-button.active {
    color: var(--accent);
  }
  .icon-button.danger:hover:not(:disabled) {
    color: var(--danger);
  }
  .icon-button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .icon-button:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
