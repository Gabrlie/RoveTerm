<script lang="ts">
  // "Start at login": RoveTerm starts minimized when the user logs in, so scheduled tasks run.
  // The operating system holds the setting; hidden where it is not available (mobile).
  import { onMount } from 'svelte';
  import Switch from '../ui/Switch.svelte';
  import { errorText, t } from '../i18n';
  import { autostartGet, autostartSet } from '../ipc';
  import { notify } from '../ui/toast.svelte';

  let enabled = $state<boolean | null>(null);

  async function set(value: boolean): Promise<void> {
    try {
      await autostartSet(value);
      enabled = value;
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  onMount(() => {
    autostartGet()
      .then((value) => (enabled = value))
      .catch((error: unknown) => console.warn('start at login unavailable', error));
  });
</script>

{#if enabled !== null}
  <div class="form-row">
    <span class="form-label">{t('settings.autostart')}<small>{t('settings.autostartHint')}</small></span>
    <Switch label={t('settings.autostart')} checked={enabled} onchange={(value) => void set(value)} />
  </div>
{/if}
