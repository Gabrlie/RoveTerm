<script lang="ts">
  import IconButton from '../ui/IconButton.svelte';
  import Icon from '../ui/Icon.svelte';
  import { HOME_TAB, workspace } from '../workspace/workspace.svelte';
  import { t } from '../i18n';
  import type { IconName } from '../ui/icons';

  interface Props {
    onnew: () => void;
    sidebarOpen: boolean;
    /** Shows the collapsed sidebar again. */
    onsidebar: () => void;
    rightOpen: boolean;
    bottomOpen: boolean;
    ontoggleright: () => void;
    ontogglebottom: () => void;
  }

  let { onnew, sidebarOpen, onsidebar, rightOpen, bottomOpen, ontoggleright, ontogglebottom }: Props = $props();

  function title(tabId: string): string {
    const tab = workspace.tabs.find((candidate) => candidate.id === tabId);
    return tab ? (workspace.panes[tab.focused]?.title ?? '') : '';
  }

  function tabIcon(tabId: string): IconName {
    const tab = workspace.tabs.find((candidate) => candidate.id === tabId);
    const kind = tab ? workspace.panes[tab.focused]?.target.kind : undefined;
    return kind === 'local' ? 'terminal' : kind === 'sftp' ? 'files' : 'server';
  }

  /** Connection state of the tab's focused pane, shown as a dot. */
  function tabState(tabId: string): 'live' | 'connecting' | 'failed' | 'ended' {
    const tab = workspace.tabs.find((candidate) => candidate.id === tabId);
    const pane = tab ? workspace.panes[tab.focused] : undefined;
    if (!pane) return 'ended';
    if (pane.failure) return 'failed';
    if (pane.exited) return 'ended';
    return pane.sessionId ? 'live' : 'connecting';
  }

  function onAuxClick(event: MouseEvent, tabId: string): void {
    if (event.button === 1) workspace.closeTab(tabId);
  }
</script>

<div class="tabbar" class:bare-corner={!sidebarOpen} role="tablist" data-tauri-drag-region>
  {#if !sidebarOpen}
    <IconButton icon="sidebar" label={t('action.sidebar.toggle')} onclick={onsidebar} />
  {/if}
  <div
    class="tab home"
    class:active={workspace.activeTabId === HOME_TAB}
    role="tab"
    tabindex="0"
    aria-selected={workspace.activeTabId === HOME_TAB}
    title={t('home.title')}
    onclick={() => workspace.activate(HOME_TAB)}
    onkeydown={(event) => event.key === 'Enter' && workspace.activate(HOME_TAB)}
  >
    <Icon name="home" size={14} />
  </div>
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
      <span class="glyph">
        <Icon name={tabIcon(tab.id)} size={14} />
        <span class="dot {tabState(tab.id)}"></span>
      </span>
      <span class="title">{title(tab.id)}</span>
      <IconButton icon="close" label={t('action.tab.close')} small onclick={(event) => { event.stopPropagation(); workspace.closeTab(tab.id); }} />
    </div>
  {/each}
  <IconButton icon="add" label={t('tab.new')} onclick={onnew} />
  <span class="spacer" data-tauri-drag-region></span>
  <IconButton icon="panelBottom" label={t('action.panel.bottom')} active={bottomOpen} onclick={ontogglebottom} />
  <IconButton icon="panelRight" label={t('action.panel.right')} active={rightOpen} onclick={ontoggleright} />
</div>

<style>
  .tabbar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 44px;
    padding: 0 var(--space-2);
    background: var(--material-bar);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: inset 0 -1px 0 var(--separator);
    overflow-x: auto;
    scrollbar-width: none;
  }
  /* With the sidebar collapsed the macOS window buttons sit over the tab bar. */
  :global(:root[data-platform='mac']) .tabbar.bare-corner {
    padding-left: 84px;
  }
  .tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 128px;
    max-width: 220px;
    height: 30px;
    padding: 0 var(--space-1) 0 var(--space-3);
    border-radius: var(--radius-sm);
    color: var(--fg-muted);
    cursor: default;
    user-select: none;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
    animation: tab-in var(--dur) var(--ease);
  }
  @keyframes tab-in {
    from {
      opacity: 0;
      transform: scale(0.94);
    }
  }
  .spacer {
    flex: 1;
  }
  .tab.home {
    flex: none;
    min-width: 0;
    width: 30px;
    padding: 0;
    justify-content: center;
    animation: none;
  }
  .tab:hover:not(.active) {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .tab.active {
    background: var(--surface);
    color: var(--fg);
    box-shadow:
      0 0 0 0.5px var(--separator),
      var(--shadow-sm);
  }
  .tab:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .tab :global(.icon-button) {
    opacity: 0;
  }
  .tab:hover :global(.icon-button),
  .tab.active :global(.icon-button),
  .tab :global(.icon-button:focus-visible) {
    opacity: 1;
  }
  .glyph {
    position: relative;
    display: inline-flex;
  }
  .dot {
    position: absolute;
    right: -3px;
    bottom: -2px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    box-shadow: 0 0 0 1.5px var(--surface);
  }
  .dot.live {
    background: var(--success);
  }
  .dot.connecting {
    background: var(--warning);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .dot.failed {
    background: var(--danger);
  }
  .dot.ended {
    background: var(--fg-subtle);
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
</style>
