<script lang="ts">
  import IconButton from '../ui/IconButton.svelte';
  import Icon from '../ui/Icon.svelte';
  import { workspace } from '../workspace/workspace.svelte';
  import { t } from '../i18n';

  let { onnew }: { onnew: () => void } = $props();

  function title(tabId: string): string {
    const tab = workspace.tabs.find((candidate) => candidate.id === tabId);
    return tab ? (workspace.panes[tab.focused]?.title ?? '') : '';
  }

  function onAuxClick(event: MouseEvent, tabId: string): void {
    if (event.button === 1) workspace.closeTab(tabId);
  }
</script>

<div class="tabbar" role="tablist">
  {#each workspace.tabs as tab (tab.id)}
    <div
      class="tab"
      class:active={tab.id === workspace.activeTabId}
      role="tab"
      tabindex="0"
      aria-selected={tab.id === workspace.activeTabId}
      onclick={() => workspace.activate(tab.id)}
      onkeydown={(event) => event.key === 'Enter' && workspace.activate(tab.id)}
      onauxclick={(event) => onAuxClick(event, tab.id)}
    >
      <Icon name={workspace.panes[tab.focused]?.target.kind === 'local' ? 'terminal' : 'server'} size={14} />
      <span class="title">{title(tab.id)}</span>
      <IconButton icon="close" label={t('action.tab.close')} small onclick={(event) => { event.stopPropagation(); workspace.closeTab(tab.id); }} />
    </div>
  {/each}
  <IconButton icon="add" label={t('action.tab.newLocal')} onclick={onnew} />
</div>

<style>
  .tabbar {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    height: 36px;
    padding: 0 var(--space-2);
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 120px;
    max-width: 220px;
    height: 28px;
    padding: 0 var(--space-1) 0 var(--space-3);
    border-radius: var(--radius-sm);
    color: var(--fg-muted);
    cursor: pointer;
    user-select: none;
  }
  .tab:hover {
    background: var(--bg-sunken);
  }
  .tab.active {
    background: var(--bg);
    color: var(--fg);
    box-shadow: 0 0 0 1px var(--border);
  }
  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
