<script lang="ts">
  import { onMount } from 'svelte';
  import { getVersion } from '@tauri-apps/api/app';
  import { t, type MessageKey } from '../i18n';
  import type { Theme } from '../ipc';
  import { capabilities, isMobile } from '../platform';
  import { sync } from '../sync/sync.svelte';
  import { updater } from '../update/updater.svelte';
  import { pushBackHandler } from '../ui/backStack';
  import Icon from '../ui/Icon.svelte';
  import { settingsError } from './edit.svelte';
  import { prefs } from './prefs.svelte';
  import { groupSettingsSections, settingsSections, type SettingsSection } from './sections';
  import SettingsSectionContent from './SettingsSectionContent.svelte';

  interface Props {
    section?: SettingsSection | null;
    onselect?: (section: SettingsSection | null) => void;
    onopentailscale?: () => void;
  }

  let { section = $bindable(null), onselect, onopentailscale }: Props = $props();
  let appVersion = $state('');
  const entries = settingsSections({ isMobile, biometricVault: capabilities.biometricVault, includeShortcuts: false });
  const groups = groupSettingsSections(entries);
  const selected = $derived(entries.find((entry) => entry.id === section));
  const updateAvailable = $derived(updater.info?.available ?? false);
  const currentVersion = $derived(updater.info?.current_version || appVersion);
  const themeLabels: Record<Theme, MessageKey> = {
    light: 'settings.themeLight',
    dark: 'settings.themeDark',
    system: 'settings.themeSystem',
  };

  function select(next: SettingsSection | null): void {
    if (onselect) onselect(next);
    else section = next;
  }

  function summary(id: SettingsSection): string {
    switch (id) {
      case 'appearance':
        return t(themeLabels[prefs.value.appearance.theme]);
      case 'terminal':
        return `${prefs.value.terminal.font_size}px`;
      case 'sync':
        if (sync.status?.ready) return t('mobile.settings.syncReady');
        return t(sync.status?.configured ? 'mobile.settings.syncPending' : 'mobile.settings.notConfigured');
      case 'security':
        return t(prefs.device.vault_biometric ? 'mobile.settings.enabled' : 'mobile.settings.disabled');
      case 'about':
        if (updateAvailable) return t('mobile.settings.updateAvailable');
        return currentVersion ? `v${currentVersion}` : '';
      default:
        return '';
    }
  }

  $effect(() => {
    if (selected) return pushBackHandler(() => select(null));
  });

  onMount(() => {
    void sync.refreshStatus().catch(() => {
      // Keep the last known status when the refresh fails.
    });
    void getVersion()
      .then((value) => { appVersion = value; })
      .catch((error: unknown) => console.warn('version unavailable', error));
  });
</script>

<section class="settings-page" aria-label={t('settings.title')}>
  {#if selected}
    <header class="navigation">
      <button type="button" class="plain back-button" onclick={() => select(null)}>
        <Icon name="back" size={18} />
        {t('settings.title')}
      </button>
      <h1>{t(selected.label)}</h1>
    </header>
    {#key selected.id}
      <div class="content detail">
        {#if settingsError.text}
          <p class="problem" role="alert">{settingsError.text}</p>
        {/if}
        <SettingsSectionContent section={selected.id} {onopentailscale} onopenlogs={() => select('logs')} />
      </div>
    {/key}
  {:else}
    <header class="navigation root-title">
      <h1>{t('settings.title')}</h1>
    </header>
    <div class="content">
      {#each groups as group (group.id)}
        <div class="section-group">
          <h2 class="section-label">{t(group.label)}</h2>
          <div class="form-group">
            {#each group.items as entry (entry.id)}
              {@const value = summary(entry.id)}
              <button type="button" class="section-row" onclick={() => select(entry.id)}>
                <span class="glyph" style:--tint="var(--sys-{entry.tint})"><Icon name={entry.icon} size={16} /></span>
                <span class="label">{t(entry.label)}</span>
                {#if value}<span class="summary">{value}</span>{/if}
                {#if entry.id === 'about' && updateAvailable}
                  <span class="update-dot" aria-hidden="true"></span>
                {/if}
                <span class="chevron"><Icon name="chevron" size={15} /></span>
              </button>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .settings-page {
    display: flex;
    flex: 1;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--surface);
  }
  .navigation {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    min-height: 48px;
    padding: 0 var(--space-4);
    background: var(--material-bar);
    box-shadow: inset 0 -0.5px 0 var(--separator);
  }
  h1 {
    margin: 0;
    font-size: var(--text-title);
    font-weight: 700;
  }
  .root-title {
    padding-block: var(--space-3);
  }
  .root-title h1 {
    font-size: var(--text-large);
  }
  .back-button {
    flex: none;
    min-height: 44px;
    margin-inline-start: calc(-1 * var(--space-2));
    color: var(--accent);
  }
  .content {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-5);
    min-height: 0;
    padding: var(--space-3) var(--space-4) var(--space-6);
    overflow-y: auto;
  }
  .detail {
    gap: var(--space-2);
  }
  .content > :global(*) {
    flex-shrink: 0;
  }
  .detail :global(.section-label) {
    margin-top: var(--space-3);
  }
  .section-label {
    margin-bottom: var(--space-2);
  }
  .section-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: 50px;
    padding: var(--space-2) var(--space-3);
    border: 0;
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .section-row + .section-row {
    box-shadow: inset 0 0.5px 0 var(--separator);
  }
  .section-row:active {
    background: var(--fill-pressed);
  }
  .section-row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .glyph {
    display: grid;
    flex: none;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    background: var(--tint);
    color: white;
  }
  .label {
    flex: 1;
    min-width: 0;
  }
  .summary {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    text-align: end;
  }
  .chevron {
    display: inline-flex;
    flex: none;
    color: var(--fg-muted);
  }
  .update-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--danger);
  }
  .problem {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--text-footnote);
    overflow-wrap: anywhere;
  }
</style>
