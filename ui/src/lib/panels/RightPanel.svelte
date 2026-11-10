<script lang="ts">
  // The right-hand panel: server status of the focused pane's remote, and snippets.
  import IconButton from '../ui/IconButton.svelte';
  import StatusPanel from './StatusPanel.svelte';
  import SnippetsPanel from './SnippetsPanel.svelte';
  import { t } from '../i18n';
  import { remoteSourceOf, sourceKey, workspace, hostOf } from '../workspace/workspace.svelte';
  import { hosts } from '../hosts/hosts.svelte';

  type Page = 'status' | 'snippets';

  let { page = $bindable('status'), onclose }: { page?: Page; onclose: () => void } = $props();

  const source = $derived.by(() => {
    const pane = workspace.focusedPane;
    if (!pane) return null;
    // A deleted host has nothing to sample: show the skeleton, not a lookup error.
    const hostId = hostOf(pane.target);
    if (hostId && hosts.loaded && !hosts.find(hostId)) return null;
    return remoteSourceOf(pane.target);
  });
</script>

<aside class="panel">
  <div class="tabs">
    <div class="segmented" role="tablist">
    <button type="button" role="tab" aria-selected={page === 'status'} class:active={page === 'status'} onclick={() => (page = 'status')}>{t('status.title')}</button>
    <button type="button" role="tab" aria-selected={page === 'snippets'} class:active={page === 'snippets'} onclick={() => (page = 'snippets')}>{t('snippets.title')}</button>
    </div>
    <span class="spacer"></span>
    <IconButton icon="close" label={t('common.close')} small onclick={onclose} />
  </div>
  <div class="body">
    {#if page === 'status'}
      {#key source ? sourceKey(source) : ''}
        <StatusPanel {source} visible={true} />
      {/key}
    {:else}
      <SnippetsPanel />
    {/if}
  </div>
</aside>

<style>
  .panel {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--material-sidebar);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    height: 44px;
    padding: 0 var(--space-2) 0 var(--space-3);
  }
  .spacer {
    flex: 1;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
</style>
