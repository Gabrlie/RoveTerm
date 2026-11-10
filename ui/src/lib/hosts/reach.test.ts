import { test } from 'node:test';
import assert from 'node:assert/strict';
import { reachKey } from './reach.ts';
import type { HostEntry } from '../ipc';

function host(id: string, hostname: string, route: HostEntry['route'] = { kind: 'direct' }): HostEntry {
  return { id, name: id, group: null, hostname, port: 22, username: 'root', auth: { kind: 'password' }, tags: [], route } as unknown as HostEntry;
}

test('the key changes with the address, the route and a jump host', () => {
  const hosts = new Map([
    ['a', host('a', 'a.example', { kind: 'jump', host: 'j' })],
    ['j', host('j', 'jump.example')],
  ]);
  const find = (id: string): HostEntry | undefined => hosts.get(id);
  const before = reachKey('a', find);
  assert.equal(before, 'a=root@a.example:22 via [j=root@jump.example:22]');
  hosts.set('j', host('j', 'other-jump.example'));
  assert.notEqual(reachKey('a', find), before);
  hosts.set('a', host('a', 'a.example'));
  assert.equal(reachKey('a', find), 'a=root@a.example:22');
  assert.equal(reachKey('gone', find), 'gone');
});

test('jump loops end', () => {
  const hosts = new Map([
    ['a', host('a', 'a', { kind: 'jump', host: 'b' })],
    ['b', host('b', 'b', { kind: 'jump', host: 'a' })],
  ]);
  assert.match(reachKey('a', (id) => hosts.get(id)), /via [ab]$|via \[/);
});
