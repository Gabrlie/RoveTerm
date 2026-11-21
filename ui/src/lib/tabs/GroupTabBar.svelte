<script lang="ts">
  // Tabs of one editor group: home, the group's tabs, a button to show home ("new tab"), and
  // splitting or closing the group.
  import IconButton from '../ui/IconButton.svelte';
  import Icon from '../ui/Icon.svelte';
  import { hostOf, workspace, type Group } from '../workspace/workspace.svelte';
  import type { PaneId } from '../layout/layout';
  import { t } from '../i18n';
  import type { IconName } from '../ui/icons';
  import { hosts } from '../hosts/hosts.svelte';
  import { editors } from '../files/editors.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { justDragged, pressTab, tabDrag } from './tabDrag.svelte';

  interface Props {
    group: Group;
    /** The group has focus: its current tab is highlighted. */
    focused: boolean;
    /** Whether the group can be closed (other groups remain). */
    closable: boolean;
  }

  let { group, focused, closable }: Props = $props();

  function tabIcon(paneId: PaneId): IconName {
    const kind = workspace.panes[paneId]?.target.kind;
    if (kind === 'editor') return 'fileCode';
    return kind === 'local' ? 'terminal' : kind === 'sftp' ? 'files' : kind === 'fleet' ? 'bot' : 'server';
  }

  /** Editor tabs use filenames. Saved terminal and file browser tabs follow host renames. */
  function tabTitle(paneId: PaneId): string {
    const pane = workspace.panes[paneId];
    if (!pane) return '';
    const target = pane.target;
    if (target.kind === 'fleet') return t('fleet.title');
    if (target.kind === 'editor') return target.title;
    const hostId = hostOf(target);
    if (hostId && target.kind !== 'local' && target.kind !== 'quick') return hosts.find(hostId)?.name || target.title;
    return pane.title;
  }

  function tabTooltip(paneId: PaneId): string | undefined {
    const pane = workspace.panes[paneId];
    if (pane?.target.kind !== 'editor') return pane?.title;
    const { draftId, path } = pane.target;
    const label = editors.find(draftId)?.label;
    return label ? `${label} — ${path}` : path;
  }

  /** The status dot tracks unsaved changes in editor tabs and connectivity in remote tabs. */
  function tabState(paneId: PaneId): 'live' | 'connecting' | 'failed' | 'ended' | 'dirty' | null {
    const pane = workspace.panes[paneId];
    if (!pane) return 'ended';
    if (pane.target.kind === 'editor') {
      const draft = editors.find(pane.target.draftId);
      return draft && draft.text !== draft.original ? 'dirty' : null;
    }
    if (pane.failure) return 'failed';
    if (pane.exited) return 'ended';
    return pane.sessionId || pane.target.kind === 'sftp' || pane.target.kind === 'fleet' ? 'live' : 'connecting';
  }

  let menu = $state<{ x: number; y: number; paneId: PaneId } | null>(null);

  /** The tab menu: closing, splitting, moving into a new group, and reordering. */
  function menuItems(paneId: PaneId): MenuItem[] {
    const index = group.tabs.indexOf(paneId);
    const alone = group.tabs.length === 1;
    const groups = workspace.groupIds;
    const next = groups[(groups.indexOf(group.id) + 1) % groups.length];
    const items: MenuItem[] = [
      { label: t('common.close'), icon: 'close', onselect: () => void workspace.requestClosePane(paneId) },
      { label: t('tab.closeOthers'), onselect: () => void workspace.requestCloseTabsBeside(paneId, 'others') },
      { label: t('tab.closeRight'), onselect: () => void workspace.requestCloseTabsBeside(paneId, 'right') },
    ];
    if (workspace.panes[paneId]?.target.kind !== 'editor') {
      items.push(
        { label: t('action.pane.splitRight'), icon: 'splitRight', separated: true, onselect: () => workspace.splitTab(paneId, 'row') },
        { label: t('action.pane.splitDown'), icon: 'splitDown', onselect: () => workspace.splitTab(paneId, 'column') },
      );
    }
    if (!alone) {
      items.push(
        { label: t('tab.moveRightGroup'), separated: true, onselect: () => workspace.moveTabToNewGroup(paneId, 'row') },
        { label: t('tab.moveDownGroup'), onselect: () => workspace.moveTabToNewGroup(paneId, 'column') },
      );
    }
    if (next && next !== group.id) {
      items.push({ label: t('tab.moveNextGroup'), separated: alone, onselect: () => workspace.moveTab(paneId, next) });
    }
    if (index > 0) items.push({ label: t('tab.moveLeft'), separated: true, onselect: () => workspace.shiftTab(paneId, -1) });
    if (index < group.tabs.length - 1) items.push({ label: t('tab.moveRight'), separated: index === 0, onselect: () => workspace.shiftTab(paneId, 1) });
    return items;
  }

  function openMenu(event: MouseEvent, paneId: PaneId): void {
    event.preventDefault();
    workspace.activate(group.id, paneId);
    menu = { x: event.clientX, y: event.clientY, paneId };
  }

  /** Where a dragged tab would land in this bar, as an insertion index. */
  const dropIndex = $derived(tabDrag.target?.kind === 'tabs' && tabDrag.target.groupId === group.id ? tabDrag.target.index : null);

  function onAuxClick(event: MouseEvent, paneId: PaneId): void {
    if (event.button === 1) void workspace.requestClosePane(paneId);
  }

</script>

<div class="tabbar" class:focused role="tablist" data-tabbar={group.id}>
  <div
    class="tab home"
    class:active={group.active === null}
    role="tab"
    tabindex="0"
    aria-selected={group.active === null}
    title={t('home.title')}
    onclick={() => workspace.activate(group.id, null)}
    onkeydown={(event) => event.key === 'Enter' && workspace.activate(group.id, null)}
  >
    <Icon name="home" size={14} />
  </div>
  {#each group.tabs as paneId, index (paneId)}
    {@const state = tabState(paneId)}
    <div
      class="tab"
      class:active={paneId === group.active}
      class:dragging={tabDrag.paneId === paneId}
      class:drop-before={dropIndex === index}
      class:drop-after={dropIndex === group.tabs.length && index === group.tabs.length - 1}
      role="tab"
      tabindex="0"
      aria-selected={paneId === group.active}
      data-tab-index={index}
      onpointerdown={(event) => pressTab(event, paneId)}
      oncontextmenu={(event) => openMenu(event, paneId)}
      onclick={() => !justDragged() && workspace.activate(group.id, paneId)}
      onkeydown={(event) => event.key === 'Enter' && workspace.activate(group.id, paneId)}
      onauxclick={(event) => onAuxClick(event, paneId)}
      title={tabTooltip(paneId)}
    >
      <span class="glyph">
        <Icon name={tabIcon(paneId)} size={14} />
        {#if state}
          <span class="dot {state}" aria-label={state === 'dirty' ? t('editor.unsaved') : undefined}></span>
        {/if}
      </span>
      <span class="title">{tabTitle(paneId)}</span>
      <IconButton icon="close" label={t('action.tab.close')} small onclick={(event) => { event.stopPropagation(); void workspace.requestClosePane(paneId); }} />
    </div>
  {/each}
  <IconButton icon="add" label={t('tab.new')} onclick={() => workspace.activate(group.id, null)} />
  <span class="spacer"></span>
  <IconButton icon="splitRight" label={t('action.pane.splitRight')} small onclick={() => { workspace.focusGroup(group.id); workspace.splitFocused('row'); }} />
  {#if closable}
    <IconButton icon="close" label={t('tab.closeGroup')} small onclick={() => void workspace.requestCloseGroupWithTabs(group.id)} />
  {/if}
</div>
{#if menu}
  {@const menuPane = menu.paneId}
  <ContextMenu x={menu.x} y={menu.y} items={menuItems(menuPane)} onclose={() => (menu = null)} />
{/if}

<style>
  .tab.dragging {
    opacity: 0.45;
  }
  /* Where a dragged tab would be inserted. */
  .tab.drop-before::before,
  .tab.drop-after::after {
    content: '';
    position: absolute;
    top: 4px;
    bottom: 4px;
    width: 2px;
    border-radius: 1px;
    background: var(--accent);
  }
  .tab.drop-before::before {
    left: -2px;
  }
  .tab.drop-after::after {
    right: -2px;
  }
  .tabbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    height: 40px;
    padding: 0 var(--space-2);
    background: var(--material-bar);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: inset 0 -1px 0 var(--separator);
    overflow-x: auto;
    scrollbar-width: none;
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
  .dot.dirty {
    background: var(--accent);
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
  .spacer {
    flex: 1;
  }
  /* The current tab of a group that does not have focus is drawn quieter. */
  .tabbar:not(.focused) .tab.active {
    background: color-mix(in srgb, var(--surface) 60%, transparent);
    color: var(--fg-muted);
    box-shadow: 0 0 0 0.5px var(--separator);
  }
</style>
