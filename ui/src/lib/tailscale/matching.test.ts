import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { HostEntry } from '../ipc';
import type { TailscalePeer } from './types';
import { dismissKey, findConversionCandidates, matchTailscalePeer, matchTailscalePeerForAddress, normalizeHostAddress } from './matching.ts';

const peer = (id: string, name: string, dns_name: string, addresses: string[]): TailscalePeer => ({ id, name, dns_name, addresses, online: true });
const hosts = (id: string, hostname: string, route: HostEntry['route'] = { kind: 'direct' }): HostEntry => ({ id, name: id, hostname, username: 'root', port: 22, route, auth: { method: 'password' }, has_secret: false, group: null, tags: [], created_at: 0, updated_at: 0, last_connected_at: null, os: null });

const peers = [
  peer('v4', 'router', 'router.tail.example.', ['100.64.0.1']),
  peer('v6', 'storage', 'storage.tail.example', ['fd7a:115c:a1e0::10']),
  peer('v6b', 'backup', 'backup.tail.example', ['fd7a:115c:a1e0:0:0:0:0:10']),
  peer('short', 'nas', 'nas.tail.example', ['100.64.0.4']),
  peer('short2', 'nas', 'nas2.tail.example', ['100.64.0.5']),
];

test('normalizes IPv4, trailing dots, bracketed and equivalent IPv6 addresses', () => {
  assert.equal(normalizeHostAddress(' 100.64.0.1. '), '100.64.0.1');
  assert.equal(normalizeHostAddress('[FD7A:115C:A1E0:0:0:0:0:10]'), 'fd7a:115c:a1e0::10');
  assert.equal(matchTailscalePeer(hosts('v4host', '100.64.0.1'), peers)?.id, 'v4');
  assert.equal(matchTailscalePeer(hosts('v6host', '[FD7A:115C:A1E0:0:0:0:0:10]'), peers)?.id, 'v6');
  assert.equal(matchTailscalePeerForAddress('fd7a:115c:a1e0::10', peers)?.id, 'v6');
});

test('matches MagicDNS case-insensitively and ignores a trailing dot', () => {
  assert.equal(matchTailscalePeer(hosts('dns', 'ROUTER.TAIL.EXAMPLE.'), peers)?.id, 'v4');
});

test('short names match only when unique and localhost never matches by name', () => {
  assert.equal(matchTailscalePeer(hosts('unique', 'storage'), peers)?.id, 'v6');
  assert.equal(matchTailscalePeer(hosts('ambiguous', 'nas'), peers), null);
  assert.equal(matchTailscalePeer(hosts('localhost', 'localhost'), [peer('local', 'localhost', '', ['100.64.8.8'])]), null);
});

test('does not match a host reached through Tailscale, jump, or proxy routes', () => {
  for (const route of [{ kind: 'tailscale' } as const, { kind: 'jump', host: 'jump' } as const, { kind: 'proxy', protocol: 'http', host: 'proxy', port: 8080 } as const]) {
    assert.equal(matchTailscalePeer(hosts('routed', '100.64.0.1', route), peers), null);
  }
});

test('conversion candidates omit dismissed host and peer matches', () => {
  const allHosts = [hosts('one', '100.64.0.1'), hosts('two', 'router.tail.example')];
  assert.deepEqual(findConversionCandidates(allHosts, peers, new Set()).map(({ host, peer }) => [host.id, peer.id]), [['one', 'v4'], ['two', 'v4']]);
  assert.deepEqual(findConversionCandidates(allHosts, peers, new Set([dismissKey('one', 'v4')])).map(({ host }) => host.id), ['two']);
  assert.equal(dismissKey('one', 'v4'), 'one:v4');
});
