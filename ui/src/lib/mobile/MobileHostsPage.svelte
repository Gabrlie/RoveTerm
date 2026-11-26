<script lang="ts">
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import HostIcon from '../hosts/HostIcon.svelte';
  import { hostMenuItems, type HostMenuHandlers } from '../hosts/hostMenu';
  import { hosts } from '../hosts/hosts.svelte';
  import { hostSubtitle } from '../hosts/route';
  import { errorText, t } from '../i18n';
  import type { HostEntry } from '../ipc';
  import type { PaneId } from '../layout/layout';
  import ContextMenu from '../ui/ContextMenu.svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { notify } from '../ui/toast.svelte';
  import { findConversionCandidates } from '../tailscale/matching';
  import { tailscale } from '../tailscale/tailscale.svelte';
  import { vault } from '../vault/vault.svelte';
  import type { PaneState } from '../workspace/workspace.svelte';
  import MobilePageHeader from './MobilePageHeader.svelte';

  interface Props extends HostMenuHandlers {
    fleetWaiting: number;
    backgroundTerminal?: PaneState;
    onedit: (host: HostEntry | null) => void;
    onquick: () => void;
    onfleet: () => void;
    onresume: (paneId: PaneId) => void;
    ondisconnect: (paneId: PaneId) => void;
  }

  let {
    fleetWaiting, backgroundTerminal, onconnect, onfiles, onforwards, onedit,
    onquick, onfleet, onresume, ondisconnect, onnotice,
  }: Props = $props();
  let query = $state('');
  let menu = $state<{ host: HostEntry; x: number; y: number } | null>(null);

  const visibleHosts = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    return hosts.items
      .filter((host) => !needle || [host.name, host.hostname, host.username, host.group ?? '', ...host.tags].join(' ').toLowerCase().includes(needle))
      .sort((a, b) => (b.last_connected_at ?? 0) - (a.last_connected_at ?? 0) || a.name.localeCompare(b.name));
  });
  const backgroundTarget = $derived(backgroundTerminal?.target);
  const backgroundHost = $derived(backgroundTarget?.kind === 'ssh' ? hosts.find(backgroundTarget.hostId) : undefined);
  const conversionCandidates = $derived(
    tailscale.connected ? findConversionCandidates(hosts.items, tailscale.peers, tailscale.dismissed) : [],
  );
  const firstConversionCandidate = $derived(conversionCandidates[0] ?? null);

  async function convertCandidate(candidate: (typeof conversionCandidates)[number]): Promise<void> {
    try {
      await tailscale.convertHost(candidate.host, candidate.peer);
      notify(t('tailscale.converted', { name: candidate.host.name }), 'success');
    } catch (error) {
      onnotice(errorText(error));
    }
  }

  async function convertAllCandidates(): Promise<void> {
    let converted = 0;
    for (const candidate of conversionCandidates) {
      try {
        await tailscale.convertHost(candidate.host, candidate.peer);
        converted += 1;
      } catch (error) {
        onnotice(errorText(error));
      }
    }
    if (converted) notify(t('tailscale.convertedCount', { count: converted }), 'success');
  }

  function dismissAllCandidates(): void {
    for (const candidate of conversionCandidates) {
      tailscale.dismissCandidate(candidate.host.id, candidate.peer.id);
    }
  }

  function lastSeen(host: HostEntry): string {
    if (!host.last_connected_at) return t('home.never');
    const minutes = Math.round((Date.now() - host.last_connected_at) / 60_000);
    if (minutes < 1) return t('home.justNow');
    if (minutes < 60) return t('home.minutesAgo', { n: minutes });
    const hours = Math.round(minutes / 60);
    if (hours < 24) return t('home.hoursAgo', { n: hours });
    return t('home.daysAgo', { n: Math.round(hours / 24) });
  }

  function showMenu(event: MouseEvent, host: HostEntry): void {
    const bounds = (event.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { host, x: bounds.right, y: bounds.bottom };
  }
</script>

<div class="page">
  <MobilePageHeader title={t('mobile.hosts.title')} large>
    {#snippet actions()}
      <IconButton icon="quickConnect" label={t('action.host.quickConnect')} onclick={onquick} />
      <IconButton icon="add" label={t('host.new')} onclick={() => onedit(null)} />
    {/snippet}
  </MobilePageHeader>

  <div class="content">
    {#if !vault.unlocked}
      <div class="unlock">
        <Icon name="lock" size={18} />
        <span>{t(vault.keyLost ? 'vault.keyLost' : 'mobile.hosts.unlockVault')}</span>
        <button type="button" class="secondary" onclick={() => void vault.unlock().catch((error) => onnotice(errorText(error)))}>
          {t('mobile.hosts.unlockAction')}
        </button>
      </div>
    {/if}

    <div class="search">
      <Icon name="search" size={17} />
      <input class="bare" bind:value={query} placeholder={t('host.search')} aria-label={t('host.search')} autocomplete="off" spellcheck="false" />
      {#if query}
        <IconButton icon="close" label={t('ai.historyClearSearch')} onclick={() => (query = '')} />
      {/if}
    </div>

    {#if hosts.items.length > 0}
      {#if conversionCandidates.length > 0}
        <section class="ts-convert" aria-label={t('tailscale.matches')}>
          <div class="ts-convert-head">
            <span class="ts-convert-icon"><Icon name="network" size={16} /></span>
            <div class="ts-convert-copy">
              <strong>{t('tailscale.conversionTitle')}</strong>
              {#if conversionCandidates.length === 1 && firstConversionCandidate}
                <small>{t('tailscale.conversionSingle', { host: firstConversionCandidate.host.name, hostname: firstConversionCandidate.host.hostname, peer: firstConversionCandidate.peer.name || firstConversionCandidate.peer.dns_name })}</small>
              {:else}
                <small>{t('tailscale.conversionCount', { count: conversionCandidates.length, names: conversionCandidates.slice(0, 3).map(({ host }) => host.name).join(', ') })}</small>
              {/if}
            </div>
          </div>
          <div class="ts-convert-actions">
            {#if conversionCandidates.length === 1 && firstConversionCandidate}
              {@const candidate = firstConversionCandidate}
              <button type="button" class="secondary" onclick={() => tailscale.dismissCandidate(candidate.host.id, candidate.peer.id)}>{t('tailscale.ignore')}</button>
              <button type="button" class="primary" onclick={() => void convertCandidate(candidate)}>{t('tailscale.convertHost')}</button>
            {:else}
              <button type="button" class="secondary" onclick={dismissAllCandidates}>{t('tailscale.dismissAll')}</button>
              <button type="button" class="primary" onclick={() => void convertAllCandidates()}>{t('tailscale.convertAll')}</button>
            {/if}
          </div>
        </section>
      {/if}
      <div class="form-group">
        <button type="button" class="row fleet" onclick={onfleet}>
          <span class="glyph"><Icon name="bot" size={17} /></span>
          <span class="copy">
            <strong>{t('fleet.title')}</strong>
            <small>{t('mobile.hosts.fleetHint')}</small>
          </span>
          {#if fleetWaiting > 0}<span class="badge waiting" role="img" aria-label={t('status.waiting', { n: fleetWaiting })}>{fleetWaiting}</span>{/if}
          <span class="trailing"><Icon name="chevron" size={16} /></span>
        </button>
      </div>
      {#if visibleHosts.length > 0}
        <div class="form-group">
          {#each visibleHosts as host (host.id)}
            {@const count = forwardStatuses.running(host.id)}
            <div class="host-row">
              <button type="button" class="row" onclick={() => onconnect(host)}>
                <HostIcon os={host.os} size={32} />
                <span class="copy">
                  <span class="name">
                    <strong>{host.name}</strong>
                    {#if count > 0}<span class="badge" title={t('host.forwards')} role="img" aria-label={t('mobile.tools.activeCount', { n: count })}>{count}</span>{/if}
                  </span>
                  <small>{hostSubtitle(host)} · {lastSeen(host)}</small>
                </span>
              </button>
              <div class="row-action">
                <IconButton icon="more" label={t('host.more')} active={menu?.host.id === host.id} onclick={(event) => showMenu(event, host)} />
              </div>
            </div>
          {/each}
        </div>
      {:else}
        <p class="no-match">{t('host.noMatch')}</p>
      {/if}
    {:else}
      <div class="empty">
        <span class="empty-glyph"><Icon name="server" size={28} /></span>
        <p>{t('home.empty')}</p>
        <button type="button" class="primary" onclick={() => onedit(null)}>{t('host.new')}</button>
      </div>
    {/if}
  </div>

  {#if backgroundTerminal}
    {@const pane = backgroundTerminal}
    {@const status = pane.failure ? 'failed' : pane.exited ? 'ended' : pane.sessionId ? 'live' : 'connecting'}
    <div class="resume">
      <button type="button" class="row" onclick={() => onresume(pane.id)}>
        {#if backgroundHost}<HostIcon os={backgroundHost.os} size={28} />{/if}
        <span class="status-dot {status}" role="img" aria-label={t(`status.state.${status}`)}></span>
        <span class="copy">
          <strong>{pane.title}</strong>
          <small>{t('mobile.hosts.resume')}</small>
        </span>
        <span class="trailing"><Icon name="chevron" size={16} /></span>
      </button>
      <div class="row-action">
        <IconButton icon="close" label={t('mobile.hosts.disconnect')} danger onclick={() => ondisconnect(pane.id)} />
      </div>
    </div>
  {/if}
</div>

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    items={hostMenuItems(menu.host, { onconnect, onfiles, onforwards, onedit, onnotice })}
    onclose={() => (menu = null)}
  />
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .content {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--m-section-gap, 18px);
    min-height: 0;
    padding: var(--space-3) var(--m-page-x, 16px) var(--space-6);
    overflow-y: auto;
  }
  .content > * {
    flex-shrink: 0;
  }
  .search, .unlock {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
    color: var(--fg-muted);
  }
  .search {
    height: var(--control-height, 36px);
    min-height: 36px;
    background: var(--fill);
  }
  .search input {
    flex: 1;
    width: 0;
    height: 100%;
  }
  .search:focus-within {
    box-shadow: var(--focus-ring);
  }
  .search :global(.icon-button) {
    margin-inline-end: calc(-1 * var(--space-2));
  }
  .unlock {
    min-height: var(--m-row-single, 46px);
    padding-block: var(--space-2);
    background: var(--accent-soft);
    color: var(--fg);
    font-size: var(--text-footnote);
  }
  .unlock span {
    flex: 1;
  }
  .unlock button {
    flex: none;
    height: var(--m-btn-inline-height, 30px);
    min-width: 0;
    padding: 0 12px;
    border-radius: 8px;
    font-size: calc(12.5px * var(--ui-scale));
  }
  .ts-convert {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--m-card-radius, 12px);
    background: var(--surface-raised);
    box-shadow: 0 0 0 0.5px var(--separator), var(--shadow-sm);
  }
  .ts-convert-head {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .ts-convert-icon {
    display: grid;
    flex: none;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .ts-convert-copy {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .ts-convert-copy strong {
    font-size: calc(13.5px * var(--ui-scale));
    white-space: normal;
  }
  .ts-convert-copy small {
    white-space: normal;
    line-height: 1.4;
  }
  .ts-convert-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .ts-convert-actions button {
    height: var(--m-btn-inline-height, 30px);
    min-width: 0;
    padding: 0 12px;
    border-radius: 8px;
    font-size: calc(12.5px * var(--ui-scale));
  }
  .row {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    min-height: var(--m-row-two-line, 54px);
    padding: 9px 14px;
    border: 0;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--m-text-body, 14.5px);
    text-align: start;
    cursor: pointer;
  }
  .row:active {
    background: var(--fill-pressed);
  }
  .row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .host-row, .resume {
    display: flex;
    align-items: stretch;
    min-width: 0;
  }
  .host-row + .host-row {
    box-shadow: inset 0 0.5px 0 var(--separator);
  }
  .row-action {
    flex: none;
    display: flex;
    align-items: center;
    padding-inline-end: 6px;
  }
  .copy {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  strong, small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  small, .trailing, .no-match, .empty {
    color: var(--fg-muted);
    font-size: var(--m-text-sub, 12.5px);
  }
  .glyph, .empty-glyph {
    display: grid;
    flex: none;
    place-items: center;
    width: var(--m-icon-tile, 32px);
    height: var(--m-icon-tile, 32px);
    border-radius: var(--space-2);
    background: color-mix(in srgb, var(--sys-purple) 18%, transparent);
    color: var(--sys-purple);
  }
  .badge {
    flex: none;
    min-width: 16px;
    padding-inline: var(--space-1);
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }
  .waiting {
    background: var(--danger);
    color: var(--accent-fg);
  }
  .empty, .no-match {
    text-align: center;
  }
  .empty {
    display: flex;
    flex: 1 0 auto;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-6) 0;
  }
  .empty p {
    margin: 0;
  }
  .empty-glyph {
    width: 52px;
    height: 52px;
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    color: var(--accent);
  }
  .resume {
    flex: none;
    background: var(--material-bar);
    box-shadow: inset 0 0.5px 0 var(--separator);
  }
  .resume .row {
    min-height: var(--m-header-large, 52px);
    padding-inline-start: var(--m-page-x, 16px);
  }
  .status-dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fg-subtle);
  }
  .status-dot.live { background: var(--success); }
  .status-dot.connecting { background: var(--warning); }
  .status-dot.failed { background: var(--danger); }
</style>
