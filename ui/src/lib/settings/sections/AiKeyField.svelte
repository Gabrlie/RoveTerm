<script lang="ts">
  import { onDestroy } from 'svelte';
  import { errorText, t } from '../../i18n';
  import IconButton from '../../ui/IconButton.svelte';

  interface Props {
    label: string;
    hint: string;
    stored: boolean;
    reveal?: () => Promise<string | null>;
    disabled?: boolean;
    value: string;
  }

  let { label, hint, stored, reveal, disabled = false, value = $bindable() }: Props = $props();
  let visible = $state(false);
  let saved = $state('');
  let loading = $state(false);
  let error = $state('');
  let request = 0;

  function hide(): void {
    request += 1;
    visible = false;
    saved = '';
    loading = false;
    error = '';
  }

  export function clear(): void {
    hide();
    value = '';
  }

  onDestroy(clear);

  async function toggle(): Promise<void> {
    if (visible) {
      hide();
      return;
    }
    visible = true;
    if (value || !stored || !reveal) return;
    const current = ++request;
    loading = true;
    try {
      const key = await reveal();
      if (current === request) saved = key ?? '';
    } catch (failure) {
      if (current === request) {
        hide();
        error = errorText(failure);
      }
    } finally {
      if (current === request) loading = false;
    }
  }
</script>

<div class="key-field">
  <label>
    <span>{label}</span>
    <input
      class="mono"
      type={visible ? 'text' : 'password'}
      value={value || saved}
      oninput={(event) => { value = event.currentTarget.value; saved = ''; }}
      autocomplete="off"
      spellcheck="false"
      placeholder={stored ? '••••••••' : ''}
      disabled={disabled || loading}
    />
    <small>{hint}</small>
  </label>
  <div class="toggle">
    <IconButton icon={visible ? 'eyeOff' : 'eye'} label={t(visible ? 'ai.keyHide' : 'ai.keyShow')} {disabled} onclick={() => void toggle()} />
  </div>
  {#if error}<p class="error-message" role="alert">{error}</p>{/if}
</div>

<style>
  .key-field { position: relative; min-width: 0; }
  label { display: grid; gap: var(--space-1); font-weight: 500; font-size: var(--text-footnote); }
  input { width: 100%; padding-right: calc(var(--control-height) + var(--space-1)); }
  .mono { font-family: var(--font-mono); }
  .toggle { position: absolute; right: var(--space-1); top: calc(1lh + var(--space-1)); font-size: var(--text-footnote); }
  small { color: var(--fg-muted); }
  .error-message { margin: var(--space-1) 0 0; color: var(--danger); font-size: var(--text-footnote); }
</style>
