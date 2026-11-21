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

  interface Props {
    onclose: () => void;
    onconnect: (host: HostEntry) => void;
    onmanagekeys: () => void;
  }

  let { onclose, onconnect, onmanagekeys }: Props = $props();
  let view = $state(emptyDiscovery());
  let query = $state('');
  let addresses = $state<Record<string, string>>(Object.create(null));
  let selected = $state<{ peer: TailscalePeer; address: string; connect: boolean } | null>(null);
  const requests = new RefreshGate();
  const peers = $derived(filterPeers(view.snapshot?.peers ?? [], query));

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
</script>

<Dialog title={t('tailscale.title')} {onclose} width={720}>
  <div class="toolbar">
    <div class="search"><Icon name="search" size={14} /><input class="bare" bind:value={query} placeholder={t('tailscale.search')} spellcheck="false" /></div>
    <button type="button" class="secondary" onclick={refresh} disabled={view.refreshing}><Icon name="refresh" size={14} />{t('tailscale.refresh')}</button>
  </div>
  {#if view.snapshot}
    <p class="status" aria-live="polite">{t(`tailscale.state.${view.snapshot.state}` as MessageKey)}</p>
    <p class="updated">{t('tailscale.updated', { time: new Date(view.snapshot.captured_at).toLocaleString() })}</p>
  {:else if view.refreshing}
    <p class="status" aria-live="polite">{t('tailscale.loading')}</p>
  {/if}
  {#if view.stale}<p class="stale" role="status">{t('tailscale.stale')}</p>{/if}
  {#if view.error}<p class="error" role="alert">{view.error}</p>{/if}
  <p class="hint">{t('tailscale.reachHint')}</p>
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
  <HostEditor host={null} prefill={{ name: selected.peer.name || selected.peer.dns_name, hostname: selected.address, username: '' }} reuseExisting onsaved={saved} onreuse={reuse} onclose={() => selected = null} submitLabel={selected.connect ? t('tailscale.saveAndConnect') : undefined} {onmanagekeys} />
{/if}

<style>
  .toolbar { display: flex; gap: var(--space-2); }
  .toolbar > button { display: inline-flex; align-items: center; gap: var(--space-2); flex: none; }
  .search { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--space-2); padding: 0 var(--space-2); border-radius: var(--radius-sm); background: var(--fill); color: var(--fg-muted); }
  .search input { width: 100%; min-width: 0; outline: none; color: var(--fg); }
  .status, .updated, .hint, .stale, .empty, .dns { margin: 0; }
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
</style>
