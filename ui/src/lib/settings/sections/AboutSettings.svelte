<script lang="ts">
  import { onMount } from 'svelte';
  import { getVersion } from '@tauri-apps/api/app';
  import Icon from '../../ui/Icon.svelte';
  import Switch from '../../ui/Switch.svelte';
  import { errorText, t } from '../../i18n';
  import { logsReveal, type UpdateChannel } from '../../ipc';
  import { isAndroid, isMobile } from '../../platform';
  import { notify } from '../../ui/toast.svelte';
  import AutostartRow from '../AutostartRow.svelte';
  import { editDevice } from '../edit.svelte';
  import { prefs } from '../prefs.svelte';
  import { updater } from '../../update/updater.svelte';
  import { effectiveUpdateChannel, isPrereleaseVersion } from '../../update/updaterLogic';

  let { onopenlogs }: { onopenlogs?: () => void } = $props();
  let version = $state('');

  const activeChannel = $derived(effectiveUpdateChannel(prefs.device.updates.channel, version));
  const channelBusy = $derived(
    updater.phase === 'downloading' || updater.phase === 'installing' || (updater.phase === 'ready' && !isAndroid),
  );

  async function selectChannel(next: UpdateChannel): Promise<void> {
    if (channelBusy) return;
    if (prefs.device.updates.channel === next) return;
    await editDevice((draft) => {
      draft.updates.channel = next;
    });
    updater.resetForChannelChange();
    void updater.check({ silent: false, interactive: false });
  }

  const updateSummary = $derived.by(() => {
    switch (updater.phase) {
      case 'checking':
        return t('update.checking');
      case 'available':
        return updater.info?.version
          ? updater.skipped
            ? t('update.availableSkipped', { version: updater.info.version })
            : t('update.availableSummary', { version: updater.info.version })
          : t('update.checkHint');
      case 'downloading':
        return updater.percent !== null
          ? t('update.downloadingPercent', { percent: updater.percent })
          : t('update.downloading');
      case 'installing':
        return t('update.installing');
      case 'ready':
        return updater.info?.version
          ? t(isAndroid ? 'update.androidReadySummary' : 'update.readySummary', { version: updater.info.version })
          : t(isAndroid ? 'update.androidReadyBanner' : 'update.readyBanner');
      case 'up_to_date':
        return t('update.upToDate');
      case 'error':
        return updater.error || t('update.checkHint');
      default:
        return prefs.device.updates.skipped_version
          ? t('update.skippedHint', { version: prefs.device.updates.skipped_version })
          : t('update.checkHint');
    }
  });

  onMount(() => {
    getVersion()
      .then((value) => (version = value))
      .catch((error: unknown) => console.warn('version unavailable', error));
  });
</script>

<div class="about">
  <div class="mark"><Icon name="terminal" size={34} /></div>
  <h3>RoveTerm</h3>
  <p class="version">
    {t('settings.version')} {version || '…'}
    {#if isPrereleaseVersion(version)}
      <span class="beta-badge">{t('update.prereleaseBadge')}</span>
    {/if}
  </p>
  <p>{t('settings.aboutText')}</p>
</div>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('update.channel')}<small>{t('update.channelHint')}</small></span>
    <div class="segmented" role="group" aria-label={t('update.channel')}>
      <button
        type="button"
        disabled={channelBusy}
        aria-pressed={activeChannel === 'stable'}
        onclick={() => void selectChannel('stable')}
      >
        {t('update.channelStable')}
      </button>
      <button
        type="button"
        disabled={channelBusy}
        aria-pressed={activeChannel === 'beta'}
        onclick={() => void selectChannel('beta')}
      >
        {t('update.channelBeta')}
      </button>
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">{t('update.autoCheck')}<small>{t('update.autoCheckHint')}</small></span>
    <Switch
      label={t('update.autoCheck')}
      checked={prefs.device.updates.auto_check}
      onchange={(value) => void editDevice((draft) => (draft.updates.auto_check = value))}
    />
  </div>
  <div class="form-row">
    <span class="form-label">
      {t('update.sectionTitle')}
      <small class:update-error={updater.phase === 'error'}>{updateSummary}</small>
    </span>
    <div class="log-buttons">
      {#if prefs.device.updates.skipped_version}
        <button type="button" class="plain" onclick={() => void updater.clearSkipped()}>
          {t('update.clearSkipped')}
        </button>
      {/if}
      {#if updater.phase === 'ready' || updater.phase === 'available' || updater.phase === 'downloading' || updater.phase === 'installing'}
        <button type="button" class="primary" onclick={() => updater.openDialog()}>
          {updater.phase === 'ready' && !isAndroid ? t('update.restartNow') : t('update.viewUpdate')}
        </button>
      {:else}
        <button
          type="button"
          class="secondary"
          disabled={updater.phase === 'checking'}
          onclick={() => void updater.check({ silent: false, interactive: true })}
        >
          {updater.phase === 'checking' ? t('update.checking') : t('update.checkNow')}
        </button>
      {/if}
    </div>
  </div>
</div>
<div class="form-group">
  <AutostartRow />
  <div class="form-row">
    <span class="form-label">{t('settings.logs')}<small>{t('settings.logsHint')}</small></span>
    <div class="log-buttons">
      {#if onopenlogs}
        <button type="button" class="secondary" onclick={onopenlogs}>{t('settings.viewLogs')}</button>
      {/if}
      <button type="button" class="secondary" onclick={() => void logsReveal().catch((error: unknown) => notify(errorText(error), 'error'))}>{t(isMobile ? 'logs.share' : 'action.help.logs')}</button>
    </div>
  </div>
</div>
<p class="form-note">{t('settings.aboutLicenses')}</p>

<style>
  .log-buttons {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .update-error {
    color: var(--danger);
  }
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
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    font-variant-numeric: tabular-nums;
  }
  .beta-badge {
    padding: 1px 6px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-caption);
    font-weight: 600;
  }
  p {
    margin: var(--space-2) 0 0;
    color: var(--fg);
  }
  @media (max-width: 767px) {
    .about {
      padding: var(--space-4) var(--space-3) var(--space-3);
    }
    .log-buttons {
      flex-wrap: wrap;
    }
  }
</style>
