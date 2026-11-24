<script lang="ts">
  import HostIcon from './HostIcon.svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { hosts } from './hosts.svelte';
  import { vault } from '../vault/vault.svelte';
  import { ask } from '../ui/confirm.svelte';
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte';
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import { errorText, t } from '../i18n';
  import type { HostEntry } from '../ipc';
  import { hostMenuItems } from './hostMenu';
  import { hostAddress, hostSubtitle } from './route';
  import { isMobile } from '../platform';
  import { tailscale } from '../tailscale/tailscale.svelte';
  import type { MessageKey } from '../i18n';
  import { viewport } from '../layout/viewport.svelte';

  interface Props {
    drawer?: boolean;
    onclose?: () => void;
    onconnect: (host: HostEntry) => void;
    onfiles: (host: HostEntry) => void;
    onforwards: (host: HostEntry) => void;
    onedit: (host: HostEntry | null) => void;
    onnotice: (message: string) => void;
    onsettings: () => void;
    ontailscale: () => void;
    /** Opens the key library. */
    onkeys: () => void;
  }

  let { drawer = false, onclose, onconnect, onfiles, onforwards, onedit, onnotice, onsettings, ontailscale, onkeys }: Props = $props();

  const tapToConnect = $derived(viewport.coarse || viewport.compact || drawer);
  let menu = $state<{ x: number; y: number; host: HostEntry } | null>(null);
  const tailscaleStatusLabel = $derived(t(tailscale.stateLabelKey() as MessageKey));

  function openMenu(event: MouseEvent, host: HostEntry): void {
    event.preventDefault();
    event.stopPropagation();
    menu = { x: event.clientX, y: event.clientY, host };
  }

  const menuItems = (host: HostEntry): MenuItem[] => hostMenuItems(host, { onconnect, onfiles, onforwards, onedit, onnotice });

  async function unlock(): Promise<void> {
    if (vault.keyLost) {
      const confirmed = await ask({ title: t('vault.keyLost'), message: t('vault.confirmReset'), confirmLabel: t('vault.reset'), danger: true });
      if (!confirmed) return;
      await vault.reset().catch((error: unknown) => onnotice(errorText(error)));
      return;
    }
    const state = await vault.unlock().catch((error: unknown) => {
      onnotice(errorText(error));
      return null;
    });
    if (state?.state === 'locked') onnotice(t('vault.unlockFailed'));
  }
</script>

<aside class="sidebar" class:drawer class:touch={tapToConnect} aria-label={drawer ? t('mobile.drawer.hosts') : undefined}>
  {#if drawer && onclose}
    <header class="drawer-header">
      <span>{t('mobile.drawer.hosts')}</span>
      <IconButton icon="close" label={t('common.close')} onclick={onclose} />
    </header>
  {/if}
  <div class="search">
    <Icon name="search" size={14} />
    <input class="bare" bind:value={hosts.query} placeholder={t('host.search')} spellcheck="false" />
  </div>
  {#if !isMobile && tailscale.configured}
    <div class="integration">
      <button type="button" class="footer-button tailscale-btn" onclick={ontailscale} title={tailscaleStatusLabel}>
        <Icon name="network" size={14} />
        <span>{t('tailscale.title')}</span>
        <span class="ts-dot" class:connected={tailscale.connected} class:connecting={tailscale.connecting} aria-label={tailscaleStatusLabel}></span>
      </button>
    </div>
  {/if}
  <div class="list">
    {#each hosts.grouped as [group, entries] (group ?? '')}
      {#if group}<div class="group section-label">{group}</div>{/if}
      {#each entries as host (host.id)}
        <div
          class="host"
          role="button"
          tabindex="0"
          onclick={tapToConnect ? (event) => {
            if (!(event.target instanceof Element && event.target.closest('.actions'))) onconnect(host);
          } : undefined}
          ondblclick={tapToConnect ? undefined : () => onconnect(host)}
          oncontextmenu={(event) => openMenu(event, host)}
          onkeydown={(event) => {
            if (event.target !== event.currentTarget) return;
            if (event.key === 'Enter') onconnect(host);
            if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
              const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
              menu = { x: rect.left + 24, y: rect.bottom, host };
              event.preventDefault();
            }
          }}
          title={hostAddress(host)}
        >
          <HostIcon os={host.os} />
          <div class="text">
            <span class="name">
              <span class="label">{host.name}</span>
              {#if forwardStatuses.running(host.id) > 0}
                <span class="forwarding" title={t('forward.activeCount', { count: forwardStatuses.running(host.id) })}>
                  <Icon name="forwards" size={11} />{forwardStatuses.running(host.id)}
                </span>
              {/if}
            </span>
            <span class="address">{hostSubtitle(host)}</span>
          </div>
          <div class="actions">
            {#if !tapToConnect}
              <IconButton icon="terminal" label={t('host.connect')} small onclick={() => onconnect(host)} />
            {/if}
            <IconButton
              icon="more"
              label={t('host.more')}
              small
              onclick={(event) => {
                const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
                menu = { x: rect.left, y: rect.bottom + 2, host };
              }}
            />
          </div>
        </div>
      {/each}
    {:else}
      <p class="empty">{hosts.query ? t('host.noMatch') : t('host.empty')}</p>
    {/each}
  </div>
  {#if menu}
    <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.host)} onclose={() => (menu = null)} />
  {/if}
  <footer>
    <button type="button" class="footer-button" onclick={onkeys}>
      <Icon name="key" size={14} />
      <span>{t('keys.title')}</span>
    </button>
    <span class="spacer"></span>
    <!-- Only a vault that needs attention shows up here; an unlocked one is the normal state. -->
    {#if !vault.unlocked}
      <button type="button" class="footer-button vault" onclick={unlock} title={vault.keyLost ? t('vault.keyLost') : t('vault.locked')}>
        <Icon name="lock" size={14} />
        <span>{vault.keyLost ? t('vault.keyLost') : t('vault.locked')}</span>
      </button>
    {/if}
    <IconButton icon="settings" label={t('action.settings.open')} small onclick={onsettings} />
  </footer>
</aside>

<style>
  .sidebar {
    width: 248px;
    flex: none;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-lg);
    background: var(--material-sidebar);
    box-shadow: var(--panel-shadow);
    overflow: hidden;
  }
  .sidebar.drawer {
    width: min(300px, 85vw);
    height: 100%;
    border-radius: 0 var(--radius-lg) var(--radius-lg) 0;
  }
  .drawer-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-1) var(--space-2) 0 var(--space-3);
    font-weight: 600;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-height);
    margin: var(--space-3) var(--space-3) var(--space-2);
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
    transition: box-shadow var(--dur-fast) var(--ease-out);
  }
  .search:focus-within {
    background: var(--surface);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 70%, transparent),
      var(--focus-ring);
  }
  .search input {
    flex: 1;
    min-width: 0;
    color: var(--fg);
    outline: none;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0 var(--space-2) var(--space-2);
  }
  .integration { padding: 0 var(--space-3) var(--space-2); }
  .integration .footer-button { width: 100%; }
  .tailscale-btn { width: 100%; }
  .ts-dot { margin-left: auto; width: 8px; height: 8px; border-radius: 50%; flex: none; background: var(--danger); }
  .ts-dot.connected { background: var(--success); box-shadow: 0 0 0 2px var(--success-soft); }
  .ts-dot.connecting { background: var(--warning); box-shadow: 0 0 0 2px var(--warning-soft); animation: ts-pulse 1.2s ease-in-out infinite; }
  @keyframes ts-pulse { 50% { opacity: 0.35; } }
  @media (prefers-reduced-motion: reduce) { .ts-dot.connecting { animation: none; } }
  .group {
    padding: var(--space-3) var(--space-2) var(--space-1);
  }
  .host {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 40px;
    padding: 0 var(--space-1) 0 var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--fg-muted);
    cursor: default;
    user-select: none;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .host:hover {
    background: var(--fill-hover);
  }
  .host:focus-visible {
    background: var(--accent);
    color: color-mix(in srgb, var(--accent-fg) 80%, transparent);
    outline: none;
  }
  .host:focus-visible .name,
  .host:focus-visible .forwarding {
    color: var(--accent-fg);
  }
  .host:focus-visible :global(.icon-button) {
    color: var(--accent-fg);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .name {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    color: var(--fg);
    font-weight: 500;
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .forwarding {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-left: var(--space-1);
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-caption);
    font-weight: 600;
  }
  .address {
    font-size: var(--text-caption);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: none;
  }
  .host:hover .actions,
  .host:focus-within .actions,
  .touch .actions {
    display: flex;
  }
  .touch .host {
    min-height: 48px;
  }
  .touch .actions :global(.icon-button) {
    width: 44px;
    height: 44px;
  }
  .empty {
    padding: var(--space-3);
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    text-align: center;
  }
  footer {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-2);
    box-shadow: inset 0 1px 0 var(--separator);
  }
  .spacer {
    flex: 1;
  }
  .footer-button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    height: 26px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--text-footnote);
    cursor: pointer;
  }
  .footer-button span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .footer-button:hover {
    background: var(--fill-hover);
    color: var(--fg);
  }
  .vault :global(svg) {
    color: var(--warning);
  }
  /* The key library keeps its label; a vault warning shrinks first. */
  .footer-button:not(.vault) {
    flex: none;
  }
</style>
