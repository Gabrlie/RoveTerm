<script lang="ts" module>
  export type SettingsSection = 'appearance' | 'terminal' | 'files' | 'status' | 'sync' | 'shortcuts' | 'about';
</script>

<script lang="ts">
  // Preferences, grouped like the system settings: sections on the left, grouped rows on the
  // right. Every change applies and is stored at once.
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import type { IconName } from '../ui/icons';
  import { t, type MessageKey } from '../i18n';
  import { settingsError } from './edit.svelte';
  import AppearanceSettings from './sections/AppearanceSettings.svelte';
  import TerminalSettings from './sections/TerminalSettings.svelte';
  import FilesSettings from './sections/FilesSettings.svelte';
  import StatusSettings from './sections/StatusSettings.svelte';
  import ShortcutsSettings from './sections/ShortcutsSettings.svelte';
  import SyncSettings from './sections/SyncSettings.svelte';
  import AboutSettings from './sections/AboutSettings.svelte';

  let { initial = 'appearance', onclose }: { initial?: SettingsSection; onclose: () => void } = $props();

  type Section = SettingsSection;

  const SECTIONS: { id: Section; label: MessageKey; icon: IconName; tint: string }[] = [
    { id: 'appearance', label: 'settings.section.appearance', icon: 'palette', tint: 'blue' },
    { id: 'terminal', label: 'settings.section.terminal', icon: 'terminal', tint: 'graphite' },
    { id: 'files', label: 'settings.section.files', icon: 'folder', tint: 'teal' },
    { id: 'status', label: 'settings.section.status', icon: 'pulse', tint: 'green' },
    { id: 'sync', label: 'settings.section.sync', icon: 'cloud', tint: 'teal' },
    { id: 'shortcuts', label: 'settings.section.shortcuts', icon: 'keyboard', tint: 'orange' },
    { id: 'about', label: 'settings.section.about', icon: 'info', tint: 'purple' },
  ];

  // svelte-ignore state_referenced_locally
  let section = $state<Section>(initial);
  settingsError.text = '';
</script>

<Dialog title={t('settings.title')} {onclose} width={780}>
  <div class="settings">
    <nav aria-label={t('settings.title')}>
      {#each SECTIONS as entry (entry.id)}
        <button type="button" class:active={section === entry.id} aria-current={section === entry.id ? 'page' : undefined} onclick={() => (section = entry.id)}>
          <span class="glyph" style:--tint="var(--sys-{entry.tint})"><Icon name={entry.icon} size={13} /></span>
          {t(entry.label)}
        </button>
      {/each}
    </nav>
    <div class="pane">
      <h2>{t(SECTIONS.find((entry) => entry.id === section)?.label ?? 'settings.title')}</h2>
      {#if section === 'appearance'}
        <AppearanceSettings />
      {:else if section === 'terminal'}
        <TerminalSettings />
      {:else if section === 'files'}
        <FilesSettings />
      {:else if section === 'status'}
        <StatusSettings />
      {:else if section === 'sync'}
        <SyncSettings />
      {:else if section === 'shortcuts'}
        <ShortcutsSettings />
      {:else}
        <AboutSettings />
      {/if}
    </div>
  </div>
  {#snippet actions()}
    {#if settingsError.text}<p class="error problem">{settingsError.text}</p>{/if}
    <button type="button" class="primary" onclick={onclose}>{t('settings.done')}</button>
  {/snippet}
</Dialog>

<style>
  .settings {
    display: flex;
    height: min(560px, 68vh);
    margin: 0 calc(-1 * var(--space-5));
    box-shadow:
      inset 0 1px 0 var(--separator),
      inset 0 -1px 0 var(--separator);
  }
  nav {
    flex: 0 0 184px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3) var(--space-2);
    box-shadow: inset -1px 0 0 var(--separator);
    overflow-y: auto;
  }
  nav button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 30px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: default;
  }
  nav button:hover:not(.active) {
    background: var(--fill-hover);
  }
  nav button.active {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .glyph {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 5px;
    background: linear-gradient(to bottom, color-mix(in srgb, var(--tint) 80%, #fff), var(--tint));
    color: #fff;
    box-shadow: 0 0.5px 1px rgb(0 0 0 / 0.15);
  }
  .pane {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-5) var(--space-6);
    overflow-y: auto;
    background: color-mix(in srgb, var(--fill) 50%, transparent);
  }
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-large);
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  /* Groups keep their height; the pane scrolls instead. */
  .pane > :global(*) {
    flex-shrink: 0;
  }
  .pane :global(.section-label) {
    margin-top: var(--space-3);
  }
  .problem {
    flex: 1;
    margin-right: auto;
  }
</style>
