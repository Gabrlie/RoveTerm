<script lang="ts">
  import type { SettingsSection } from './sections';
  import AppearanceSettings from './sections/AppearanceSettings.svelte';
  import TerminalSettings from './sections/TerminalSettings.svelte';
  import FilesSettings from './sections/FilesSettings.svelte';
  import StatusSettings from './sections/StatusSettings.svelte';
  import AiSettings from './sections/AiSettings.svelte';
  import SyncSettings from './sections/SyncSettings.svelte';
  import SecuritySettings from './sections/SecuritySettings.svelte';
  import TailscaleSettings from './sections/TailscaleSettings.svelte';
  import ShortcutsSettings from './sections/ShortcutsSettings.svelte';
  import LogsSettings from './sections/LogsSettings.svelte';
  import AboutSettings from './sections/AboutSettings.svelte';

  let { section, onopentailscale, onopenlogs }: {
    section: SettingsSection;
    onopentailscale?: () => void;
    onopenlogs?: () => void;
  } = $props();
</script>

{#if section === 'appearance'}
  <AppearanceSettings />
{:else if section === 'terminal'}
  <TerminalSettings />
{:else if section === 'files'}
  <FilesSettings />
{:else if section === 'status'}
  <StatusSettings />
{:else if section === 'ai'}
  <AiSettings />
{:else if section === 'sync'}
  <SyncSettings />
{:else if section === 'security'}
  <SecuritySettings />
{:else if section === 'tailscale'}
  <TailscaleSettings {onopentailscale} />
{:else if section === 'shortcuts'}
  <ShortcutsSettings />
{:else if section === 'logs'}
  <LogsSettings />
{:else}
  <AboutSettings {onopenlogs} />
{/if}
