import type { HostEntry } from '../ipc';
import type { TailscalePeer } from './types';

export interface TailscaleConversionCandidate {
  host: HostEntry;
  peer: TailscalePeer;
}

export function normalizeHostAddress(value: string): string {
  let normalized = value.trim().toLowerCase().replace(/\.$/, '');
  if (normalized.startsWith('[') && normalized.endsWith(']')) normalized = normalized.slice(1, -1);
  if (normalized.includes(':')) {
    try {
      return new URL(`http://[${normalized}]/`).hostname.slice(1, -1);
    } catch {
      return normalized;
    }
  }
  return normalized;
}

function addressMatch(hostname: string, peers: TailscalePeer[]): TailscalePeer | null {
  const address = normalizeHostAddress(hostname);
  if (!address) return null;
  return peers.find((peer) => peer.addresses.some((candidate) => normalizeHostAddress(candidate) === address)) ?? null;
}

export function matchTailscalePeerForAddress(hostname: string, peers: TailscalePeer[]): TailscalePeer | null {
  return addressMatch(hostname, peers);
}

export function matchTailscalePeer(host: Pick<HostEntry, 'hostname' | 'route'>, peers: TailscalePeer[]): TailscalePeer | null {
  if (host.route.kind !== 'direct') return null;
  const hostname = normalizeHostAddress(host.hostname);
  if (!hostname) return null;
  const byAddress = addressMatch(hostname, peers);
  if (byAddress) return byAddress;
  const byDnsName = peers.find((peer) => {
    const dnsName = normalizeHostAddress(peer.dns_name);
    return dnsName !== '' && dnsName === hostname;
  });
  if (byDnsName) return byDnsName;
  if (hostname.includes('.') || hostname === 'localhost') return null;
  const named = peers.filter((peer) => normalizeHostAddress(peer.name) === hostname);
  return named.length === 1 ? (named[0] ?? null) : null;
}

export function dismissKey(hostId: string, peerId: string): string {
  return `${hostId}:${peerId}`;
}

export function findConversionCandidates(hosts: HostEntry[], peers: TailscalePeer[], dismissed: ReadonlySet<string>): TailscaleConversionCandidate[] {
  const candidates: TailscaleConversionCandidate[] = [];
  for (const host of hosts) {
    const peer = matchTailscalePeer(host, peers);
    if (peer && !dismissed.has(dismissKey(host.id, peer.id))) candidates.push({ host, peer });
  }
  return candidates;
}
