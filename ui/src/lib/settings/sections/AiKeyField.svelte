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
  const inputId = $props.id();
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
  <label for={inputId}>{label}</label>
  <div class="input-wrap">
    <input
      id={inputId}
      class="mono"
      type={visible ? 'text' : 'password'}
      value={value || saved}
      oninput={(event) => { value = event.currentTarget.value; saved = ''; }}
      autocomplete="off"
      spellcheck="false"
      placeholder={stored ? '••••••••' : ''}
      disabled={disabled || loading}
    />
    <div class="toggle">
      <IconButton icon={visible ? 'eyeOff' : 'eye'} small label={t(visible ? 'ai.keyHide' : 'ai.keyShow')} {disabled} onclick={() => void toggle()} />
    </div>
  </div>
  <small>{hint}</small>
  {#if error}<p class="error-message" role="alert">{error}</p>{/if}
</div>

<style>
  .key-field { display: flex; flex-direction: column; gap: var(--space-1); min-width: 0; }
  label { color: var(--fg-muted); font-weight: 500; font-size: var(--text-footnote); }
  .input-wrap { position: relative; display: flex; align-items: center; }
  input { width: 100%; padding-right: calc(24px + var(--space-2)); }
  .mono { font-family: var(--font-mono); }
  .toggle { position: absolute; right: 3px; top: 50%; transform: translateY(-50%); display: inline-flex; }
  small { color: var(--fg-muted); font-size: var(--text-footnote); }
  .error-message { margin: 0; color: var(--danger); font-size: var(--text-footnote); }
</style>
