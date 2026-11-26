<script lang="ts">
  // On/off toggle for settings, drawn like the system switch. `label` is the accessible name when
  // the switch is not inside a <label>.
  interface Props {
    checked: boolean;
    onchange?: (checked: boolean) => void;
    disabled?: boolean;
    label?: string;
  }

  let { checked = $bindable(), onchange, disabled = false, label }: Props = $props();

  function toggle(): void {
    if (disabled) return;
    checked = !checked;
    onchange?.(checked);
  }
</script>

<button type="button" class="switch" class:on={checked} role="switch" aria-checked={checked} aria-label={label} {disabled} onclick={toggle}>
  <span class="knob"></span>
</button>

<style>
  .switch {
    position: relative;
    flex: none;
    width: 34px;
    height: 20px;
    padding: 0;
    border: none;
    border-radius: 999px;
    background: var(--fill-pressed);
    box-shadow: inset 0 0 0 0.5px var(--border);
    cursor: pointer;
    transition: background-color var(--dur) var(--ease);
  }
  .switch.on {
    background: var(--accent);
    box-shadow: none;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    box-shadow:
      0 0 0 0.5px rgb(0 0 0 / 0.08),
      0 1px 2px rgb(0 0 0 / 0.25);
    transition: transform var(--dur) var(--ease);
  }
  .switch.on .knob {
    transform: translateX(14px);
  }
  .switch:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .switch:disabled {
    opacity: 0.45;
    cursor: default;
  }
  @media (max-width: 767px) {
    .switch {
      width: 40px;
      height: 24px;
    }
    .knob {
      width: 20px;
      height: 20px;
    }
    .switch.on .knob {
      transform: translateX(16px);
    }
  }
</style>
