<script lang="ts">
  import { onMount } from 'svelte';
  import { getVersion } from '@tauri-apps/api/app';
  import Icon from '../../ui/Icon.svelte';
  import { errorText, t } from '../../i18n';
  import { logsReveal } from '../../ipc';
  import { notify } from '../../ui/toast.svelte';

  let version = $state('');

  onMount(() => {
    getVersion()
      .then((value) => (version = value))
      .catch((error: unknown) => console.warn('version unavailable', error));
  });
</script>

<div class="about">
  <div class="mark"><Icon name="terminal" size={34} /></div>
  <h3>RoveTerm</h3>
  <p class="version">{t('settings.version')} {version || '…'}</p>
  <p>{t('settings.aboutText')}</p>
</div>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('settings.logs')}<small>{t('settings.logsHint')}</small></span>
    <button type="button" class="secondary" onclick={() => void logsReveal().catch((error: unknown) => notify(errorText(error), 'error'))}>{t('action.help.logs')}</button>
  </div>
</div>
<p class="form-note">{t('settings.aboutLicenses')}</p>

<style>
  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-6) var(--space-4);
    text-align: center;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 72px;
    height: 72px;
    margin-bottom: var(--space-2);
    border-radius: 18px;
    background: linear-gradient(to bottom, color-mix(in srgb, var(--accent) 78%, #fff), var(--accent));
    box-shadow: var(--shadow-md);
    color: var(--accent-fg);
  }
  h3 {
    margin: 0;
    font-size: var(--text-large);
    font-weight: 700;
  }
  .version {
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    font-variant-numeric: tabular-nums;
  }
  p {
    margin: var(--space-2) 0 0;
    color: var(--fg);
  }
</style>
