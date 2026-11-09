<script lang="ts">
  // The right-hand panel: server status of the focused pane's remote, and snippets.
  import IconButton from '../ui/IconButton.svelte';
  import StatusPanel from './StatusPanel.svelte';
  import SnippetsPanel from './SnippetsPanel.svelte';
  import { t } from '../i18n';
  import { remoteSourceOf, sourceKey, workspace } from '../workspace/workspace.svelte';

  type Page = 'status' | 'snippets';

  let { page = $bindable('status'), onclose }: { page?: Page; onclose: () => void } = $props();

  const source = $derived.by(() => {
    const pane = workspace.focusedPane;
    return pane ? remoteSourceOf(pane.target) : null;
  });
</script>

<aside class="panel">
  <div class="tabs" role="tablist">
    <button type="button" role="tab" aria-selected={page === 'status'} class:active={page === 'status'} onclick={() => (page = 'status')}>{t('status.title')}</button>
    <button type="button" role="tab" aria-selected={page === 'snippets'} class:active={page === 'snippets'} onclick={() => (page = 'snippets')}>{t('snippets.title')}</button>
    <span class="spacer"></span>
    <IconButton icon="close" label={t('common.close')} small onclick={onclose} />
  </div>
  <div class="body">
    {#if page === 'status'}
      {#if source}
        {#key sourceKey(source)}
          <StatusPanel {source} visible={true} />
        {/key}
      {:else}
        <p class="muted">{t('status.noRemote')}</p>
      {/if}
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
    background: var(--bg-elevated);
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    height: 36px;
    padding: 0 var(--space-2);
    border-bottom: 1px solid var(--border);
  }
  .tabs button[role='tab'] {
    padding: 4px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  .tabs button.active {
    background: var(--bg-sunken);
    color: var(--fg);
  }
  .spacer {
    flex: 1;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .muted {
    margin: 0;
    padding: var(--space-3);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
</style>
