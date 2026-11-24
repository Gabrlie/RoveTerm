<script lang="ts" module>
  export type RightPage = 'status' | 'snippets' | 'ai' | 'transfers';
</script>

<script lang="ts">
  import IconButton from '../ui/IconButton.svelte';
  import StatusPanel from './StatusPanel.svelte';
  import SnippetsPanel from './SnippetsPanel.svelte';
  import TransfersPanel from './TransfersPanel.svelte';
  import { t } from '../i18n';
  import { workspace } from '../workspace/workspace.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { viewport } from '../layout/viewport.svelte';
  import { paneStatsSource, statsSourceKey } from './serverStats.svelte';

  import AiPanel from '../ai/AiPanel.svelte';

  let { page, onclose, onsettings }: { page: RightPage; onclose: () => void; onsettings: () => void } = $props();

  const TITLES: Record<RightPage, 'action.panel.status' | 'action.panel.snippets' | 'ai.title' | 'action.panel.transfers'> = {
    status: 'action.panel.status',
    snippets: 'action.panel.snippets',
    ai: 'ai.title',
    transfers: 'action.panel.transfers',
  };

  const statsSource = $derived(paneStatsSource(workspace.focusedPane, (id) => !hosts.loaded || !!hosts.find(id)));
</script>

<aside class="panel">
  <div class="tabs">
    <h3 class="title">{t(TITLES[page])}</h3>
    <span class="spacer"></span>
    <IconButton icon="close" label={t('common.close')} small onclick={onclose} />
  </div>
  <div class="body" class:fill={page === 'ai'}>
    {#if page === 'ai'}
      <AiPanel {onsettings} />
    {:else if page === 'status'}
      {#key statsSource ? statsSourceKey(statsSource) : ''}
        <StatusPanel source={statsSource} visible={true} session={workspace.focusedPane?.sessionId ?? null} />
      {/key}
    {:else if page === 'transfers'}
      <TransfersPanel />
    {:else}
      <SnippetsPanel onsent={viewport.compact ? onclose : undefined} />
    {/if}
  </div>
</aside>

<style>
  .panel {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--material-sidebar);
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
  .title {
    margin: 0;
    font-size: var(--text-body);
    font-weight: 600;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  /* The AI chat scrolls its own list and keeps its composer in view. */
  .body.fill {
    overflow: hidden;
  }
</style>
