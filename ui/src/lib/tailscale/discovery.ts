import type { HostDraft, HostEntry, HostRoute } from '../ipc';
import type { TailscalePeer, TailscaleSnapshot } from './types';

export interface DiscoveryView {
  snapshot: TailscaleSnapshot | null;
  refreshing: boolean;
  stale: boolean;
  error: string;
}

export function emptyDiscovery(): DiscoveryView {
  return { snapshot: null, refreshing: false, stale: false, error: '' };
}

export function beginRefresh(view: DiscoveryView): DiscoveryView {
  return { ...view, refreshing: true, error: '' };
}

export function finishRefresh(snapshot: TailscaleSnapshot, previous?: DiscoveryView): DiscoveryView {
  if (previous?.snapshot && ['not_installed', 'service_unavailable', 'permission_denied'].includes(snapshot.state)) {
    return { snapshot: { ...snapshot, peers: previous.snapshot.peers, captured_at: previous.snapshot.captured_at }, refreshing: false, stale: true, error: '' };
  }
  return { snapshot, refreshing: false, stale: false, error: '' };
}

export function failRefresh(view: DiscoveryView, error: string): DiscoveryView {
  return { ...view, refreshing: false, stale: view.snapshot !== null, error };
}

/** Only the latest request may publish, and destruction permanently invalidates requests. */
export class RefreshGate {
  #revision = 0;
  #active = true;

  begin(): number { return ++this.#revision; }
  accepts(revision: number): boolean { return this.#active && revision === this.#revision; }
  dispose(): void { this.#active = false; ++this.#revision; }
}

/** CLI addresses are already validated; preserve their order within each address family. */
export function preferredAddress(peer: TailscalePeer): string {
  return peer.addresses.find((address) => !address.includes(':')) ?? peer.addresses[0] ?? '';
}

export function connectionAddresses(peer: TailscalePeer): string[] {
  const ipv4 = peer.addresses.filter((address) => !address.includes(':'));
  const ipv6 = peer.addresses.filter((address) => address.includes(':'));
  const dns = peer.dns_name.replace(/\.$/, '');
  return [...new Set([...ipv4, ...ipv6, ...(dns ? [dns] : [])])];
}

export function selectedAddress(peer: TailscalePeer, selected?: string): string {
  const options = connectionAddresses(peer);
  return selected !== undefined && options.includes(selected) ? selected : preferredAddress(peer) || options[0] || '';
}

export function filterPeers(peers: TailscalePeer[], query: string): TailscalePeer[] {
  const needle = query.trim().toLowerCase();
  return needle ? peers.filter((peer) => [peer.name, peer.dns_name, peer.os ?? '', ...peer.addresses].join(' ').toLowerCase().includes(needle)) : peers;
}

function hostnameKey(hostname: string): string {
  const value = hostname.trim().toLowerCase().replace(/^\[([^\]]+)\]$/, '$1').replace(/\.$/, '');
  if (value.includes(':')) {
    try { return new URL(`http://[${value}]/`).hostname.slice(1, -1); }
    catch { /* The host form and backend validate the original input. */ }
  }
  return value;
}

function routeKey(route: HostRoute): string {
  switch (route.kind) {
    case 'jump': return JSON.stringify(['jump', route.host]);
    case 'proxy': return JSON.stringify(['proxy', route.protocol, hostnameKey(route.host), route.port]);
    case 'direct': return 'direct';
  }
}

type Target = Pick<HostDraft, 'hostname' | 'port' | 'username' | 'route'>;

/** Credentials and labels never change when an imported target already exists. */
export function findDuplicateHost(hosts: HostEntry[], draft: Target): HostEntry | undefined {
  return hosts.find((host) => hostnameKey(host.hostname) === hostnameKey(draft.hostname)
    && host.port === draft.port
    && host.username === draft.username.trim()
    && routeKey(host.route) === routeKey(draft.route));
}
