<script lang="ts">
  // The right edge of the window: toggles of the right panel's pages (one open at a time), the
  // files panel, the focused host's port forwards and the downloads, then ways to reach a new
  // host.
  import IconButton from '../ui/IconButton.svelte';
  import { t } from '../i18n';
  import type { ActionId } from '../actions';
  import type { RightPage } from '../panels/RightPanel.svelte';
  import type { IconName } from '../ui/icons';
  import { ai } from '../ai/ai.svelte';
  import DownloadsButton from '../files/DownloadsButton.svelte';

  interface Props {
    /** The right panel's open page, or `null` when closed. */
    page: RightPage | null;
    bottomOpen: boolean;
    home?: boolean;
    /** The focused tab is on a remote host: files and forwards apply. */
    remote: boolean;
    onaction: (action: ActionId) => void;
  }

  let { page, bottomOpen, home = false, remote, onaction }: Props = $props();

  const PAGES: { page: RightPage; action: ActionId; icon: IconName; label: 'action.panel.status' | 'action.ai.open' | 'action.panel.snippets' }[] = [
    { page: 'status', action: 'panel.status', icon: 'pulse', label: 'action.panel.status' },
    { page: 'ai', action: 'ai.open', icon: 'bot', label: 'action.ai.open' },
    { page: 'snippets', action: 'panel.snippets', icon: 'snippets', label: 'action.panel.snippets' },
  ];
</script>

<nav class="rail" aria-label={t('rail.label')}>
  {#each PAGES as entry (entry.page)}
    <div class="slot">
      <IconButton icon={entry.icon} label={t(entry.label)} active={page === entry.page} disabled={home} onclick={() => onaction(entry.action)} />
      {#if entry.page === 'ai' && ai.waiting > 0}<span class="badge">{ai.waiting}</span>{/if}
    </div>
  {/each}
  <span class="rule"></span>
  <IconButton icon="files" label={t('rail.files')} active={bottomOpen} disabled={home || (!remote && !bottomOpen)} onclick={() => onaction('panel.bottom')} />
  <IconButton icon="forwards" label={t('rail.forwards')} disabled={home || !remote} onclick={() => onaction('forward.manage')} />
  <DownloadsButton onopen={() => onaction('panel.transfers')} />
  <span class="spacer"></span>
  <IconButton icon="quickConnect" label={t('rail.quickConnect')} onclick={() => onaction('host.quickConnect')} />
  <IconButton icon="importConfig" label={t('rail.import')} onclick={() => onaction('host.import')} />
</nav>

<style>
  .rail {
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    width: 36px;
    padding: var(--space-1) 0 var(--space-2);
  }
  .slot {
    position: relative;
  }
  .badge {
    position: absolute;
    top: 0;
    right: -2px;
    min-width: 15px;
    height: 15px;
    padding: 0 4px;
    border-radius: var(--radius-pill);
    background: var(--danger);
    color: #fff;
    font-size: 10px;
    font-weight: 600;
    line-height: 15px;
    text-align: center;
    pointer-events: none;
  }
  .rule {
    width: 16px;
    height: 1px;
    margin: var(--space-1) 0;
    background: var(--separator);
  }
  .spacer {
    flex: 1;
  }
</style>
