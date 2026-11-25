<script lang="ts">
  import { viewport } from '../layout/viewport.svelte';
  import { onMount } from 'svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import HostEditor from '../hosts/HostEditor.svelte';
  import { tailscaleSnapshot, type HostEntry } from '../ipc';
  import { errorText, t, type MessageKey } from '../i18n';
  import { writeClipboard } from '../ui/clipboard';
  import { beginRefresh, emptyDiscovery, failRefresh, filterPeers, finishRefresh, preferredAddress, RefreshGate, type PresenceFilter } from './discovery';
  import type { TailscalePeer, TailscalePingResult } from './types';
  import { hosts } from '../hosts/hosts.svelte';
  import { tailscale } from './tailscale.svelte';
  import { findConversionCandidates } from './matching';
  import { notify } from '../ui/toast.svelte';

  interface Props {
    onclose: () => void;
    onconnect: (host: HostEntry) => void;
    onmanagekeys: () => void;
    onsettings: () => void;
  }

  let { onclose, onconnect, onmanagekeys, onsettings }: Props = $props();
  let view = $state(emptyDiscovery());
  let query = $state('');
  let presence = $state<PresenceFilter>('all');
  let selected = $state<{ peer: TailscalePeer; address: string; connect: boolean } | null>(null);
  const requests = new RefreshGate();
  const peerList = $derived(tailscale.capturedAt !== null ? tailscale.peers : view.snapshot?.peers ?? []);
  const onlineCount = $derived(peerList.filter((peer) => peer.online).length);
  const peers = $derived(filterPeers(peerList, query, presence));
  const status = $derived(tailscale.status);
  const selfNode = $derived(status.self_node);
  const selfPrimaryIp = $derived(selfNode?.addresses.find((addr) => !addr.includes(':')) ?? selfNode?.addresses[0] ?? '');
  const conversionCandidates = $derived(findConversionCandidates(hosts.items, peerList, new Set()));
  const snapshotError = $derived(
    status.state === 'error'
      ? (status.message ? errorText({ code: 'tailscale.failed', message: status.message }) : t('error.tailscale.failed'))
      : '',
  );
  const displayError = $derived(view.error && status.state !== 'error' ? view.error : (tailscale.error || snapshotError));
  const updatedTime = $derived(
    view.snapshot ? new Date(tailscale.capturedAt ?? view.snapshot.captured_at).toLocaleTimeString() : '',
  );

  onMount(() => {
    void refresh();
    return () => requests.dispose();
  });

  async function refresh(): Promise<void> {
    const request = requests.begin();
    view = beginRefresh(view);
    try {
      const snapshot = await tailscaleSnapshot();
      if (!requests.accepts(request)) return;
      view = finishRefresh(snapshot, view);
      const current = view.snapshot ?? snapshot;
      tailscale.status = current.status;
      tailscale.peers = current.peers;
      tailscale.capturedAt = current.captured_at;
    } catch (error) {
      if (requests.accepts(request)) view = failRefresh(view, errorText(error));
    }
  }

  function choose(peer: TailscalePeer, connect: boolean): void {
    const address = preferredAddress(peer) || peer.dns_name.replace(/\.$/, '');
    if (address) selected = { peer, address, connect };
  }

  function saved(host: HostEntry): void {
    const connect = selected?.connect;
    selected = null;
    onclose();
    if (connect) onconnect(host);
  }

  function reuse(host: HostEntry): void {
    const connect = selected?.connect;
    selected = null;
    onclose();
    if (connect) onconnect(host);
  }

  async function copyIp(address: string): Promise<void> {
    if (!address) return;
    try {
      await writeClipboard(address);
      notify(t('tailscale.ipCopied', { address }), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function convert(host: HostEntry, peer: TailscalePeer): Promise<void> {
    try {
      await tailscale.convertHost(host, peer);
      notify(t('tailscale.converted', { name: host.name }), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function quickAction(): Promise<void> {
    try {
      if (status.state === 'needs_login') await tailscale.openAuthUrl();
      else await tailscale.connect();
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  function pingText(ping: TailscalePingResult): string {
    if (!ping.reachable) return t('tailscale.unreachable');
    const ms = ping.latency_ms !== null ? `${ping.latency_ms} ms` : '—';
    if (ping.path.kind === 'direct') return `${t('tailscale.pathDirect')} · ${ms}`;
    if (ping.path.kind === 'relay') {
      const label = ping.path.region_code
        ? t('tailscale.pathRelayRegion', { code: ping.path.region_code.toUpperCase() })
        : t('tailscale.pathRelay');
      return `${label} · ${ms}`;
    }
    return ms;
  }

  function pingTooltip(ping: TailscalePingResult): string {
    if (!ping.reachable) return ping.message || t('tailscale.unreachable');
    if (ping.path.kind === 'direct') return `${t('tailscale.pathDirect')} (${ping.path.endpoint})`;
    if (ping.path.kind === 'relay') return `DERP ${ping.path.region_name || ping.path.region_code}`;
    return '';
  }

  function pingKindClass(ping: TailscalePingResult): string {
    if (!ping.reachable) return 'fail';
    if (ping.path.kind === 'direct') return 'direct';
    if (ping.path.kind === 'relay') return 'relay';
    return 'neutral';
  }
</script>

<Dialog presentation="page" title={t('tailscale.title')} {onclose} width={760}>
  <div class="ts-dialog" class:compact={viewport.compact}>
    <section class="status-card">
      <div class="status-main">
        <span
          class="status-dot"
          class:connected={status.state === 'running'}
          class:pending={status.state === 'starting' || status.state === 'needs_login' || status.state === 'needs_machine_auth'}
        ></span>
        <div class="status-copy">
          <div class="status-title-row">
            <strong>{selfNode ? (selfNode.name || selfNode.dns_name) : t(`tailscale.state.${status.state}` as MessageKey)}</strong>
            {#if selfNode}
              <span class="state-chip" class:connected={status.state === 'running'}>{t(`tailscale.state.${status.state}` as MessageKey)}</span>
            {/if}
            {#if selfNode?.home_derp}
              <span class="derp-chip" title="{t('tailscale.homeDerp')}: {selfNode.home_derp.name}">
                DERP {selfNode.home_derp.code.toUpperCase()} · {selfNode.home_derp.name}
              </span>
            {/if}
          </div>
          <div class="status-meta">
            {#if selfPrimaryIp}
              <button type="button" class="ip-copy" onclick={() => void copyIp(selfPrimaryIp)} title={t('tailscale.copyIp')}>
                <span>{selfPrimaryIp}</span>
                <Icon name="copy" size={11} />
              </button>
            {/if}
            {#if selfNode?.tailnet}
              <span class="tailnet">{selfNode.tailnet}</span>
            {/if}
            {#if updatedTime}
              <span class="updated">{t('tailscale.updated', { time: updatedTime })}</span>
            {:else if view.refreshing}
              <span class="updated">{t('tailscale.loading')}</span>
            {/if}
          </div>
        </div>
      </div>
      {#if status.state === 'needs_login' || status.state === 'disconnected' || status.state === 'error'}
        <button type="button" class="primary compact-btn" onclick={() => void quickAction()}>
          {status.state === 'needs_login' ? t('tailscale.openLogin') : t('tailscale.reconnect')}
        </button>
      {/if}
    </section>

    {#if status.state === 'needs_machine_auth'}
      <p class="banner info" role="status">{t('tailscale.machineAuthHint')}</p>
    {/if}
    {#if view.stale}<p class="banner warn" role="status">{t('tailscale.stale')}</p>{/if}
    {#if displayError}<p class="banner err" role="alert"><Icon name="warning" size={14} /><span>{displayError}</span></p>{/if}

    <div class="toolbar">
      <div class="search">
        <Icon name="search" size={14} />
        <input class="bare" bind:value={query} placeholder={t('tailscale.search')} spellcheck="false" />
      </div>
      <div class="segmented" role="group" aria-label={t('tailscale.title')}>
        <button type="button" aria-pressed={presence === 'all'} onclick={() => (presence = 'all')}>
          {t('tailscale.filterAll', { n: peerList.length })}
        </button>
        <button type="button" aria-pressed={presence === 'online'} onclick={() => (presence = 'online')}>
          {t('tailscale.filterOnline', { n: onlineCount })}
        </button>
      </div>
      <button
        type="button"
        class="secondary compact-btn"
        disabled={status.state !== 'running' || tailscale.pingingAll || peers.length === 0}
        onclick={() => void tailscale.pingAllPeers(peers.map((peer) => peer.id))}
      >
        <Icon name="pulse" size={14} />
        {tailscale.pingingAll ? t('tailscale.pinging') : t('tailscale.pingAll')}
      </button>
      <button type="button" class="secondary icon-btn" onclick={refresh} disabled={view.refreshing} title={t('tailscale.refresh')}>
        <Icon name="refresh" size={14} />
      </button>
      <button type="button" class="secondary icon-btn" onclick={onsettings} title={t('tailscale.settings')}>
        <Icon name="settings" size={14} />
      </button>
    </div>

    {#if conversionCandidates.length > 0}
      <details class="conversion-card">
        <summary>{t('tailscale.matches')} · {conversionCandidates.length}</summary>
        <div class="conversion-list">
          {#each conversionCandidates as candidate (candidate.host.id)}
            <div class="conversion-row">
              <span>{candidate.host.name} <small>({candidate.host.hostname})</small> → {candidate.peer.name || candidate.peer.dns_name}</span>
              <button type="button" class="secondary compact-btn" onclick={() => void convert(candidate.host, candidate.peer)}>{t('tailscale.convertHost')}</button>
            </div>
          {/each}
        </div>
      </details>
    {/if}

    <div class="devices">
      {#each peers as peer (peer.id)}
        {@const ip = preferredAddress(peer)}
        {@const ping = tailscale.pings[peer.id]}
        {@const isPinging = !!tailscale.pinging[peer.id]}
        {@const cleanDns = peer.dns_name.replace(/\.$/, '')}
        <article class="device-row">
          <div class="device-info">
            <div class="device-top">
              <span class="presence-dot" class:online={peer.online} title={peer.online ? t('tailscale.online') : t('tailscale.offline')}></span>
              <strong class="device-name">{peer.name || cleanDns || peer.id}</strong>
              {#if peer.os}<span class="os-tag">{peer.os}</span>{/if}
            </div>
            <div class="device-sub">
              {#if ip}
                <button type="button" class="ip-copy" onclick={() => void copyIp(ip)} title={t('tailscale.copyIp')}>
                  <span>{ip}</span>
                  <Icon name="copy" size={11} />
                </button>
              {/if}
              {#if cleanDns && cleanDns !== peer.name}
                <span class="dns" title={cleanDns}>{cleanDns}</span>
              {/if}
            </div>
          </div>

          <div class="device-actions">
            {#if isPinging}
              <span class="ping-pill testing">{t('tailscale.pinging')}</span>
            {:else if ping}
              <span class="ping-pill {pingKindClass(ping)}" title={pingTooltip(ping)}>
                {pingText(ping)}
              </span>
            {/if}
            <button
              type="button"
              class="secondary icon-btn"
              disabled={status.state !== 'running' || isPinging}
              onclick={() => void tailscale.pingPeer(peer.id)}
              title={t('tailscale.ping')}
            >
              <Icon name="pulse" size={13} />
            </button>
            <button type="button" class="secondary compact-btn" onclick={() => choose(peer, false)} disabled={!ip && !cleanDns}>
              {t('tailscale.saveShort')}
            </button>
            <button type="button" class="primary compact-btn" onclick={() => choose(peer, true)} disabled={!ip && !cleanDns}>
              {t('tailscale.connect')}
            </button>
          </div>
        </article>
      {:else}
        {#if view.snapshot}
          <p class="empty">
            {query.trim() ? t('tailscale.noMatch') : presence === 'online' ? t('tailscale.noOnlineMatch') : t('tailscale.empty')}
          </p>
        {/if}
      {/each}
    </div>
  </div>

  {#snippet actions()}<button class:page-action={viewport.compact} type="button" class="secondary" onclick={onclose}>{t('common.close')}</button>{/snippet}
</Dialog>

{#if selected}
  <HostEditor
    host={null}
    prefill={{ name: selected.peer.name || selected.peer.dns_name.replace(/\.$/, ''), hostname: selected.address, username: '', route: { kind: 'tailscale', peer_id: selected.peer.id } }}
    reuseExisting
    onsaved={saved}
    onreuse={reuse}
    onclose={() => (selected = null)}
    submitLabel={selected.connect ? t('tailscale.saveAndConnect') : undefined}
    {onmanagekeys}
  />
{/if}

<style>
  .ts-dialog {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .status-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: 0 0 0 0.5px var(--separator), var(--shadow-sm);
  }
  .status-main {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }
  .status-copy {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .status-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .status-title-row strong {
    font-size: var(--text-body);
    font-weight: 600;
  }
  .state-chip,
  .derp-chip {
    display: inline-flex;
    align-items: center;
    padding: 1px 7px;
    border-radius: 999px;
    font-size: var(--text-caption);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .state-chip.connected {
    background: color-mix(in srgb, var(--success) 14%, transparent);
    color: var(--success);
  }
  .derp-chip {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }
  .status-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    font-size: var(--text-caption);
    color: var(--fg-muted);
  }
  .status-dot {
    width: 10px;
    height: 10px;
    flex: none;
    border-radius: 50%;
    background: var(--danger);
  }
  .status-dot.connected {
    background: var(--success);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--success) 20%, transparent);
  }
  .status-dot.pending {
    background: var(--warning);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--warning) 20%, transparent);
  }
  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    font-size: var(--text-footnote);
  }
  .banner.info {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--fg);
  }
  .banner.warn {
    background: color-mix(in srgb, var(--warning) 14%, transparent);
    color: var(--warning);
  }
  .banner.err {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    color: var(--danger);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .search {
    flex: 1;
    min-width: 180px;
    height: var(--control-height);
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface);
    box-shadow: 0 0 0 0.5px var(--border);
    color: var(--fg-muted);
  }
  .search input {
    width: 100%;
    min-width: 0;
    outline: none;
    color: var(--fg);
  }
  .compact-btn {
    min-width: 0;
    padding: 0 var(--space-3);
    font-size: var(--text-footnote);
  }
  .icon-btn {
    min-width: 0;
    width: var(--control-height);
    padding: 0;
  }
  .conversion-card {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: 0 0 0 0.5px var(--separator);
  }
  .conversion-card summary {
    cursor: pointer;
    color: var(--fg);
    font-size: var(--text-footnote);
    font-weight: 500;
  }
  .conversion-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .conversion-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .conversion-row span {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: var(--text-footnote);
  }
  .conversion-row small {
    color: var(--fg-muted);
    font-family: var(--font-mono);
  }
  .devices {
    display: flex;
    flex-direction: column;
    max-height: 360px;
    overflow-y: auto;
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: 0 0 0 0.5px var(--separator), var(--shadow-sm);
  }
  .device-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: 10px var(--space-3);
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .device-row + .device-row {
    box-shadow: inset 0 1px 0 var(--separator);
  }
  .device-row:hover {
    background: var(--fill-hover);
  }
  .device-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1;
  }
  .device-top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .presence-dot {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: 50%;
    background: var(--fg-subtle);
  }
  .presence-dot.online {
    background: var(--success);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--success) 20%, transparent);
  }
  .device-name {
    font-size: var(--text-body);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .os-tag {
    flex: none;
    padding: 0 6px;
    border-radius: 4px;
    background: var(--fill);
    color: var(--fg-muted);
    font-size: 11px;
    line-height: 18px;
  }
  .device-sub {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding-left: 15px;
    min-width: 0;
  }
  .ip-copy {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border: none;
    border-radius: 4px;
    background: var(--fill);
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: 11.5px;
    cursor: pointer;
    flex: none;
  }
  .ip-copy:hover {
    background: var(--fill-pressed);
  }
  .ip-copy :global(svg) {
    color: var(--fg-muted);
  }
  .dns {
    color: var(--fg-muted);
    font-size: var(--text-caption);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .device-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
  }
  .ping-pill {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 8px;
    border-radius: 999px;
    font-size: 11.5px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .ping-pill.testing {
    background: var(--fill);
    color: var(--fg-muted);
  }
  .ping-pill.direct {
    background: color-mix(in srgb, var(--success) 15%, transparent);
    color: var(--success);
  }
  .ping-pill.relay {
    background: color-mix(in srgb, var(--sys-orange) 16%, transparent);
    color: var(--sys-orange);
  }
  .ping-pill.neutral {
    background: var(--fill);
    color: var(--fg);
  }
  .ping-pill.fail {
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    color: var(--danger);
  }
  .empty {
    margin: 0;
    padding: var(--space-6) var(--space-4);
    text-align: center;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }

  .page-action {
    min-height: var(--touch-target, 44px);
    height: auto;
    white-space: normal;
  }
  .compact .devices {
    max-height: none;
    overflow: visible;
  }
  .compact :is(.status-card, .device-row, .device-actions) {
    flex-wrap: wrap;
  }
  .compact .device-info {
    flex-basis: 100%;
  }
  .compact .device-actions {
    max-width: 100%;
    margin-inline-start: auto;
  }
</style>
