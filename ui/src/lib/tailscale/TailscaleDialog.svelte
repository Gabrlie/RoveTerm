<script lang="ts">
  import { onMount } from 'svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import Select from '../ui/Select.svelte';
  import HostEditor from '../hosts/HostEditor.svelte';
  import { tailscaleSnapshot, type HostEntry } from '../ipc';
  import { errorText, t, type MessageKey } from '../i18n';
  import { beginRefresh, connectionAddresses, emptyDiscovery, failRefresh, filterPeers, finishRefresh, selectedAddress, RefreshGate } from './discovery';
  import type { TailscalePeer } from './types';
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
  let addresses = $state<Record<string, string>>(Object.create(null));
  let selected = $state<{ peer: TailscalePeer; address: string; connect: boolean } | null>(null);
  const requests = new RefreshGate();
  const peerList = $derived(tailscale.capturedAt !== null ? tailscale.peers : view.snapshot?.peers ?? []);
  const peers = $derived(filterPeers(peerList, query));
  const status = $derived(tailscale.status);
  const conversionCandidates = $derived(findConversionCandidates(hosts.items, peerList, new Set()));
  const snapshotError = $derived(
    status.state === 'error'
      ? (status.message ? errorText({ code: 'tailscale.failed', message: status.message }) : t('error.tailscale.failed'))
      : ''
  );
  const displayError = $derived(view.error && status.state !== 'error' ? view.error : (tailscale.error || snapshotError));

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
      // Keep a selected address only while the current snapshot still advertises it.
      const next: Record<string, string> = Object.create(null);
      for (const peer of view.snapshot?.peers ?? []) {
        next[peer.id] = selectedAddress(peer, addresses[peer.id]);
      }
      addresses = next;
    } catch (error) {
      if (requests.accepts(request)) view = failRefresh(view, errorText(error));
    }
  }

  function choose(peer: TailscalePeer, connect: boolean): void {
    const address = selectedAddress(peer, addresses[peer.id]);
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
</script>

<Dialog title={t('tailscale.title')} {onclose} width={720}>
  <div class="toolbar">
    <div class="search"><Icon name="search" size={14} /><input class="bare" bind:value={query} placeholder={t('tailscale.search')} spellcheck="false" /></div>
    <button type="button" class="secondary" onclick={refresh} disabled={view.refreshing}><Icon name="refresh" size={14} />{t('tailscale.refresh')}</button>
    <button type="button" class="secondary" onclick={onsettings}><Icon name="settings" size={14} />{t('tailscale.settings')}</button>
  </div>
  <div class="status-header">
    <span class="status-dot" class:connected={status.state === 'running'}></span>
    <span class="status" aria-live="polite">{t(`tailscale.state.${status.state}` as MessageKey)}</span>
    {#if status.self_node}
      <span class="self-node">{status.self_node.name || status.self_node.dns_name}{#if status.self_node.addresses.length} · {status.self_node.addresses.join(', ')}{/if}</span>
    {/if}
    {#if status.state === 'needs_login' || status.state === 'disconnected' || status.state === 'error'}<button type="button" class="secondary" onclick={() => void quickAction()}>{status.state === 'needs_login' ? t('tailscale.openLogin') : t('tailscale.reconnect')}</button>{/if}
  </div>
  {#if view.snapshot}
    <p class="updated">{t('tailscale.updated', { time: new Date(tailscale.capturedAt ?? view.snapshot.captured_at).toLocaleString() })}</p>
  {:else if view.refreshing}
    <p class="updated">{t('tailscale.loading')}</p>
  {/if}
  {#if view.stale}<p class="stale" role="status">{t('tailscale.stale')}</p>{/if}
  {#if displayError}<p class="error" role="alert">{displayError}</p>{/if}
  <p class="hint">{t('tailscale.reachHint')}</p>
  {#if conversionCandidates.length > 0}
    <details class="conversion-card">
      <summary>{t('tailscale.matches')} · {conversionCandidates.length}</summary>
      {#each conversionCandidates as candidate (candidate.host.id)}
        <div class="conversion-row">
          <span>{candidate.host.name} · {candidate.host.hostname} → {candidate.peer.name || candidate.peer.dns_name}</span>
          <button type="button" class="secondary" onclick={() => void convert(candidate.host, candidate.peer)}>{t('tailscale.convertHost')}</button>
        </div>
      {/each}
    </details>
  {/if}
  <div class="devices">
    {#each peers as peer (peer.id)}
      <article class="device">
        <div class="identity"><Icon name="server" size={18} /><strong>{peer.name || peer.dns_name || peer.id}</strong>{#if peer.os}<span class="os">{peer.os}</span>{/if}</div>
        <span class="presence" class:online={peer.online}><span class="dot"></span>{peer.online ? t('tailscale.online') : t('tailscale.offline')}</span>
        {#if peer.dns_name}<p class="dns">{peer.dns_name}</p>{/if}
        <div class="connection">
          <label>{t('tailscale.address')}<Select value={selectedAddress(peer, addresses[peer.id])} options={connectionAddresses(peer).map((address) => ({ value: address, label: address }))} onchange={(value) => addresses[peer.id] = value} /></label>
          <button type="button" class="secondary" onclick={() => choose(peer, false)} disabled={!selectedAddress(peer, addresses[peer.id])}>{t('tailscale.save')}</button>
          <button type="button" class="primary" onclick={() => choose(peer, true)} disabled={!selectedAddress(peer, addresses[peer.id])}>{t('tailscale.connect')}</button>
        </div>
      </article>
    {:else}
      {#if view.snapshot}<p class="empty">{query.trim() ? t('tailscale.noMatch') : t('tailscale.empty')}</p>{/if}
    {/each}
  </div>
  {#snippet actions()}<button type="button" class="secondary" onclick={onclose}>{t('common.close')}</button>{/snippet}
</Dialog>

{#if selected}
  <HostEditor host={null} prefill={{ name: selected.peer.name || selected.peer.dns_name, hostname: selected.address, username: '', route: { kind: 'tailscale', peer_id: selected.peer.id } }} reuseExisting onsaved={saved} onreuse={reuse} onclose={() => selected = null} submitLabel={selected.connect ? t('tailscale.saveAndConnect') : undefined} {onmanagekeys} />
{/if}

<style>
  .toolbar { display: flex; gap: var(--space-2); }
  .toolbar > button { display: inline-flex; align-items: center; gap: var(--space-2); flex: none; }
  .search { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--space-2); padding: 0 var(--space-2); border-radius: var(--radius-sm); background: var(--fill); color: var(--fg-muted); }
  .search input { width: 100%; min-width: 0; outline: none; color: var(--fg); }
  .status, .updated, .hint, .stale, .empty, .dns { margin: 0; }
  .status-header { display: flex; align-items: center; gap: var(--space-2); }
  .status-dot { width: 8px; height: 8px; flex: none; border-radius: 50%; background: var(--danger); }
  .status-dot.connected { background: var(--success); box-shadow: 0 0 0 2px var(--success-soft); }
  .self-node { margin-left: auto; color: var(--fg-muted); font-size: var(--text-footnote); }
  .status { color: var(--fg); }
  .updated, .hint, .dns, .empty { font-size: var(--text-footnote); color: var(--fg-muted); }
  .stale { color: var(--warning); font-size: var(--text-footnote); }
  .devices { display: flex; flex-direction: column; gap: var(--space-3); }
  .device { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3); border-radius: var(--radius-md); background: var(--fill); }
  .identity { display: flex; align-items: center; gap: var(--space-2); min-width: 0; }
  .identity strong { overflow-wrap: anywhere; font-weight: 500; }
  .identity :global(svg) { color: var(--fg-muted); flex: none; }
  .os { margin-left: auto; color: var(--fg-muted); font-size: var(--text-caption); }
  .presence { display: inline-flex; align-items: center; gap: var(--space-1); color: var(--fg-muted); font-size: var(--text-caption); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
  .presence.online .dot { color: var(--success); }
  .dns { overflow-wrap: anywhere; }
  .connection { display: flex; align-items: end; gap: var(--space-2); flex-wrap: wrap; }
  .connection label { min-width: 180px; flex: 1; }
  .empty { padding: var(--space-4); text-align: center; }
  .conversion-card { padding: var(--space-3); border-radius: var(--radius-md); background: var(--fill); }
  .conversion-card summary { cursor: pointer; color: var(--fg-muted); font-size: var(--text-footnote); }
  .conversion-row { display: flex; align-items: center; gap: var(--space-2); padding-top: var(--space-2); }
  .conversion-row span { flex: 1; min-width: 0; overflow-wrap: anywhere; font-size: var(--text-footnote); }
</style>
