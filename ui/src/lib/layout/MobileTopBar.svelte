<script lang="ts">
  import type { ActionId } from '../actions';
  import { ai } from '../ai/ai.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { t, type MessageKey } from '../i18n';
  import type { RightPage } from '../panels/RightPanel.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import type { IconName } from '../ui/icons';
  import { vault } from '../vault/vault.svelte';
  import { hostOf, type PaneState } from '../workspace/workspace.svelte';

  interface Props {
    pane: PaneState | undefined;
    drawerOpen: boolean;
    focusedRemote: boolean;
    panelsSuspended: boolean;
    rightPage: RightPage | null;
    bottomOpen: boolean;
    onaction: (id: ActionId) => void;
    ontheme: () => void;
  }

  let { pane, drawerOpen, focusedRemote, panelsSuspended, rightPage, bottomOpen, onaction, ontheme }: Props = $props();
  let menu = $state<{ x: number; y: number } | null>(null);
  const remoteTools = $derived(focusedRemote && !panelsSuspended);
  const title = $derived.by(() => {
    if (!pane) return t('app.title');
    if (pane.target.kind === 'fleet') return t('fleet.title');
    if (pane.target.kind === 'editor') return pane.target.title;
    const hostId = hostOf(pane.target);
    return (hostId && hosts.find(hostId)?.name) || pane.title;
  });
  const status = $derived.by(() => {
    if (!pane) return null;
    if (pane.failure) return 'failed';
    if (pane.exited) return 'ended';
    if (pane.sessionId || pane.target.kind === 'sftp' || pane.target.kind === 'fleet' || pane.target.kind === 'editor') return 'live';
    return 'connecting';
  });

  function item(id: ActionId, icon: IconName, label: MessageKey): MenuItem {
    return { icon, label: t(label), onselect: () => onaction(id) };
  }

  const items = $derived([
    item('host.quickConnect', 'plug', 'action.host.quickConnect'),
    item('host.new', 'add', 'action.host.new'),
    ...(remoteTools ? [item('forward.manage', 'forwards', 'host.forwards')] : []),
    ...(!panelsSuspended ? [item('panel.snippets', 'snippets', 'action.panel.snippets')] : []),
    item('panel.transfers', 'download', 'action.panel.transfers'),
    item('palette.open', 'palette', 'action.palette.open'),
    { icon: 'sun', label: t('mobile.theme'), onselect: ontheme } satisfies MenuItem,
    item('keys.manage', 'key', 'action.keys.manage'),
    item('settings.open', 'settings', 'action.settings.open'),
  ]);
</script>

<header class="top-bar">
  <div class="badged">
    <IconButton icon="sidebar" label={t('mobile.drawer.hosts')} active={drawerOpen} onclick={() => onaction('sidebar.toggle')} />
    {#if !vault.unlocked}
      <span class="lock-dot" role="img" aria-label={t(vault.keyLost ? 'vault.keyLost' : 'vault.locked')}></span>
    {/if}
  </div>

  <div class="pane">
    {#if status}
      <span class="status {status}" role="img" aria-label={t(`mobile.pane.${status}`)}></span>
    {/if}
    <span class="title">{title}</span>
  </div>

  {#if remoteTools}
    <IconButton icon="files" label={t('action.panel.bottom')} active={bottomOpen} onclick={() => onaction('panel.bottom')} />
    <IconButton icon="pulse" label={t('action.panel.status')} active={rightPage === 'status'} onclick={() => onaction('panel.status')} />
  {/if}
  {#if !panelsSuspended}
    <div class="badged">
      <IconButton icon="bot" label={t('action.ai.open')} active={rightPage === 'ai'} onclick={() => onaction('ai.open')} />
      {#if ai.waiting > 0}
        <span class="badge" aria-label={t('status.waiting', { n: ai.waiting })}>{ai.waiting}</span>
      {/if}
    </div>
  {/if}
  <IconButton icon="more" label={t('mobile.menu.more')} active={menu !== null} onclick={(event) => {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: rect.right, y: rect.bottom };
  }} />
</header>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} {items} onclose={() => (menu = null)} />
{/if}

<style>
  .top-bar {
    display: flex;
    align-items: center;
    flex: 0 0 44px;
    background: var(--material-bar);
    box-shadow: inset 0 -0.5px 0 var(--separator);
  }
  .top-bar :global(.icon-button) {
    width: 44px;
    height: 44px;
  }
  .pane {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
    min-width: 0;
    gap: var(--space-2);
    padding-inline: var(--space-1);
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .status, .lock-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--fg-subtle);
  }
  .status.live { background: var(--success); }
  .status.connecting, .lock-dot { background: var(--warning); }
  .status.failed { background: var(--danger); }
  .badged {
    position: relative;
    display: flex;
  }
  .lock-dot, .badge {
    position: absolute;
    top: var(--space-1);
    right: var(--space-1);
    pointer-events: none;
  }
  .badge {
    min-width: 16px;
    padding-inline: var(--space-1);
    border-radius: var(--radius-pill);
    background: var(--danger);
    color: var(--accent-fg);
    text-align: center;
    font: 600 var(--text-caption) var(--font-ui);
  }
</style>
