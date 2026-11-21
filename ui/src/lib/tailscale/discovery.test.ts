import { test } from 'node:test';
import assert from 'node:assert/strict';
import { beginRefresh, connectionAddresses, emptyDiscovery, failRefresh, filterPeers, findDuplicateHost, finishRefresh, peerIpOptions, preferredAddress, SAVED_PEER_PREFIX, selectedAddress, RefreshGate } from './discovery.ts';
import type { HostEntry } from '../ipc';
import type { TailscalePeer, TailscaleSnapshot } from './types';

const peer = (id: string, addresses: string[], name = 'web'): TailscalePeer => ({ id, name, addresses, dns_name: 'web.tail.example.', online: false, os: 'linux' });
const status = (state: TailscaleSnapshot['status']['state']): TailscaleSnapshot['status'] => ({ configured: state !== 'unconfigured', state, self_node: null, auth_url: null, peer_count: 0, updated_at: 1000 });
const snapshot: TailscaleSnapshot = { status: status('running'), peers: [peer('stable-1', ['100.64.1.2'])], captured_at: 1000 };

test('connection addresses prefer IPv4 and retain IPv6 and normalized MagicDNS choices', () => {
  const device = peer('one', ['fd7a:115c:a1e0::1', '100.64.1.2', '100.64.1.2']);
  assert.equal(preferredAddress(device), '100.64.1.2');
  assert.deepEqual(connectionAddresses(device), ['100.64.1.2', 'fd7a:115c:a1e0::1', 'web.tail.example']);
  assert.equal(preferredAddress(peer('two', ['fd7a:115c:a1e0::2'])), 'fd7a:115c:a1e0::2');
  assert.equal(preferredAddress(peer('empty', [])), '');
  assert.equal(selectedAddress(device, 'web.tail.example'), 'web.tail.example');
  assert.equal(selectedAddress(device, 'removed.tail.example'), '100.64.1.2');
  assert.equal(selectedAddress(peer('dns-only', [])), 'web.tail.example');
});

test('refresh failures preserve the last snapshot and mark it stale until a success', () => {
  const initial = emptyDiscovery();
  assert.equal(failRefresh(initial, 'missing').stale, false);
  const loaded = finishRefresh(snapshot);
  const refreshing = beginRefresh(loaded);
  assert.equal(refreshing.snapshot, snapshot);
  assert.equal(refreshing.refreshing, true);
  const failed = failRefresh(refreshing, 'unavailable');
  assert.equal(failed.snapshot, snapshot);
  assert.equal(failed.stale, true);
  assert.equal(failed.refreshing, false);
  const recovered = finishRefresh({ ...snapshot, captured_at: 2000 });
  assert.equal(recovered.stale, false);
  assert.equal(recovered.error, '');
});

test('late refresh results and results after destruction cannot replace the latest state', () => {
  const gate = new RefreshGate();
  const first = gate.begin();
  const second = gate.begin();
  assert.equal(gate.accepts(first), false);
  assert.equal(gate.accepts(second), true);
  gate.dispose();
  assert.equal(gate.accepts(second), false);
  assert.equal(gate.accepts(gate.begin()), false);
});

test('error status retains the last peers and capture time while displaying the current failure', () => {
  const failure: TailscaleSnapshot = { status: { ...status('error'), message: 'unavailable' }, peers: [], captured_at: 2000 };
  const initial = finishRefresh(failure, emptyDiscovery());
  assert.equal(initial.snapshot, failure);
  assert.equal(initial.stale, false);
  const previous = finishRefresh(snapshot);
  const failed = finishRefresh(failure, beginRefresh(previous));
  assert.equal(failed.snapshot?.status.state, 'error');
  assert.equal(failed.snapshot?.status.message, 'unavailable');
  assert.equal(failed.snapshot?.peers, snapshot.peers);
  assert.equal(failed.snapshot?.captured_at, 1000);
  assert.equal(failed.stale, true);
  const recovered = finishRefresh({ ...snapshot, captured_at: 3000 }, failed);
  assert.equal(recovered.snapshot?.captured_at, 3000);
  assert.equal(recovered.stale, false);
});

test('valid CLI states replace previous devices rather than retaining stale discovery data', () => {
  for (const state of ['needs_login', 'needs_machine_auth', 'disconnected'] as const) {
    const current: TailscaleSnapshot = { status: status(state), peers: [], captured_at: 2000 };
    const result = finishRefresh(current, finishRefresh(snapshot));
    assert.equal(result.snapshot, current);
    assert.equal(result.stale, false);
  }
});

test('search matches name, DNS, system and addresses while keeping stable peer identities', () => {
  const devices = [peer('stable-1', ['100.64.1.2']), { ...peer('stable-2', ['fd7a::2']), dns_name: 'other.tail.example', os: 'windows' }];
  assert.deepEqual(filterPeers(devices, 'windows').map((entry) => entry.id), ['stable-2']);
  assert.deepEqual(filterPeers(devices, '100.64').map((entry) => entry.id), ['stable-1']);
  assert.equal(filterPeers(devices, 'WEB').length, 2);
  assert.equal(filterPeers(devices, 'missing').length, 0);
  assert.equal(filterPeers(devices, '  '), devices);
});

function host(id: string, overrides: Partial<HostEntry> = {}): HostEntry {
  return { id, name: 'existing host', hostname: 'web.tail.example', username: 'root', port: 22, route: { kind: 'direct' }, auth: { method: 'key', key: 'saved-key' }, has_secret: true, group: null, tags: [], created_at: 0, updated_at: 0, last_connected_at: null, os: null, ...overrides };
}

test('duplicate detection uses the connection target and preserves existing credentials', () => {
  const existing = host('existing');
  const draft = { name: 'new label', hostname: 'WEB.tail.example.', username: ' root ', port: 22, route: { kind: 'direct' as const } };
  assert.equal(findDuplicateHost([existing], draft), existing);
  assert.equal(existing.auth.method, 'key');
  assert.equal(existing.has_secret, true);
  assert.equal(findDuplicateHost([existing], { ...draft, username: 'Root' }), undefined);
  assert.equal(findDuplicateHost([existing], { ...draft, port: 2222 }), undefined);
  assert.equal(findDuplicateHost([existing], { ...draft, route: { kind: 'jump', host: 'bastion' } }), undefined);
});

test('duplicate detection distinguishes proxy and jump routes and normalizes IPv6 brackets', () => {
  const existing = host('v6', { hostname: 'fd7a::1', route: { kind: 'proxy', protocol: 'socks5', host: 'PROXY.example.', port: 1080 } });
  assert.equal(findDuplicateHost([existing], { hostname: '[FD7A::1]', username: 'root', port: 22, route: { kind: 'proxy', protocol: 'socks5', host: 'proxy.example', port: 1080 } }), existing);
  assert.equal(findDuplicateHost([existing], { hostname: 'fd7a:0:0:0:0:0:0:1', username: 'root', port: 22, route: { kind: 'proxy', protocol: 'socks5', host: 'proxy.example', port: 1080 } }), existing);
  assert.equal(findDuplicateHost([existing], { ...existing, route: { kind: 'proxy', protocol: 'http', host: 'proxy.example', port: 1080 } }), undefined);
  const jump = host('jump', { route: { kind: 'jump', host: 'bastion' } });
  assert.equal(findDuplicateHost([jump], { ...jump, route: { kind: 'jump', host: 'other' } }), undefined);
});

test('duplicate detection keeps Tailscale routes distinct from direct routes', () => {
  const tailscaleHost = host('tailscale', { route: { kind: 'tailscale', peer_id: 'peer-1' } });
  const target = { hostname: tailscaleHost.hostname, username: tailscaleHost.username, port: tailscaleHost.port, route: tailscaleHost.route };
  assert.equal(findDuplicateHost([tailscaleHost], { ...target, route: { kind: 'direct' } }), undefined);
  assert.equal(findDuplicateHost([tailscaleHost], target), tailscaleHost);
});

test('filterPeers supports online-only filtering alongside search queries', () => {
  const devices = [
    { ...peer('p1', ['100.64.1.1'], 'alpha'), online: true },
    { ...peer('p2', ['100.64.1.2'], 'beta'), online: false },
    { ...peer('p3', ['100.64.1.3'], 'alpha-db'), online: true },
  ];
  assert.deepEqual(filterPeers(devices, '', 'online').map((d) => d.id), ['p1', 'p3']);
  assert.deepEqual(filterPeers(devices, 'db', 'online').map((d) => d.id), ['p3']);
  assert.deepEqual(filterPeers(devices, 'beta', 'online').map((d) => d.id), []);
});

test('peerIpOptions prefers direct IPv4/IPv6 addresses and preserves unmatched saved hosts', () => {
  const devices = [
    { ...peer('p1', ['fd7a::1', '100.64.1.10'], 'prod-api'), online: true },
    { ...peer('p2', ['fd7a::2'], 'ipv6-only'), online: false },
    { ...peer('p3', [], 'no-ip'), online: true },
  ];
  const matched = peerIpOptions(devices, {
    peerId: 'p1',
    hostname: '100.64.1.10',
    formatSaved: (addr) => `Saved: ${addr}`,
  });
  assert.equal(matched.length, 2);
  assert.equal(matched[0]?.address, '100.64.1.10');
  assert.equal(matched[0]?.label, 'prod-api (100.64.1.10)');
  assert.equal(matched[1]?.address, 'fd7a::2');

  const preserved = peerIpOptions(devices, {
    peerId: 'old-peer',
    hostname: '100.64.9.9',
    formatSaved: (addr) => `Saved: ${addr}`,
  });
  assert.equal(preserved.length, 3);
  assert.equal(preserved[0]?.value, `${SAVED_PEER_PREFIX}100.64.9.9`);
  assert.equal(preserved[0]?.label, 'Saved: 100.64.9.9');
  assert.equal(preserved[0]?.peerId, 'old-peer');
});
