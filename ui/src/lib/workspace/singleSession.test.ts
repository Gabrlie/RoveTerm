import test from 'node:test';
import assert from 'node:assert/strict';
import {
  findBackgroundTerminal,
  findReusableTerminalPane,
  isTerminalTarget,
  panesToCloseForSingleTerminal,
  sameTerminalTarget,
} from './singleSession.ts';
import type { PaneState, PaneTarget } from './workspace.svelte';

const local = { kind: 'local' } as const;
const ssh = { kind: 'ssh', hostId: 'host-1', title: 'Server' } as const;
const quick = { kind: 'quick', address: 'user@server' } as const;
const sftp = { kind: 'sftp', source: { hostId: ssh.hostId }, title: 'Server files' } as const;
const editor = { kind: 'editor', source: { address: quick.address }, draftId: 'draft-1', path: '/file', title: 'file' } as const;
const fleet = { kind: 'fleet' } as const;
const exited = { code: 0, signal: null };
const failure = { code: 'ssh.connection_failed', message: 'Could not connect' };

function makePane(id: string, target: PaneTarget, state: Partial<PaneState> = {}): PaneState {
  return {
    id,
    target,
    title: id,
    password: null,
    passwordKind: 'natural',
    remember: false,
    sessionId: null,
    failure: null,
    deferred: false,
    exited: null,
    connectedAt: null,
    ...state,
  };
}

test('recognizes only local, saved-host, and quick-connect panes as terminals', () => {
  assert.deepEqual([local, ssh, quick, sftp, editor, fleet].map(isTerminalTarget), [true, true, true, false, false, false]);
});

test('matches terminal targets by host ID, quick address, or local kind', () => {
  assert.equal(sameTerminalTarget(ssh, { ...ssh, title: 'Renamed' }), true);
  assert.equal(sameTerminalTarget(ssh, { ...ssh, hostId: 'host-2' }), false);
  assert.equal(sameTerminalTarget(quick, { ...quick }), true);
  assert.equal(sameTerminalTarget(quick, { ...quick, address: 'other@server' }), false);
  assert.equal(sameTerminalTarget(local, { kind: 'local' }), true);
  assert.equal(sameTerminalTarget(ssh, { kind: 'quick', address: ssh.hostId }), false);
  assert.equal(sameTerminalTarget(sftp, sftp), false);
  assert.equal(sameTerminalTarget(fleet, fleet), false);
});

test('reuses matching healthy panes, including ones that have not connected yet', () => {
  const connecting = makePane('connecting', ssh);
  const connected = makePane('connected', quick, { sessionId: 'session-1' });
  const deferred = makePane('deferred', local, { deferred: true });
  const panes = [
    makePane('failed', ssh, { failure }),
    makePane('ended', ssh, { sessionId: 'session-2', exited }),
    connecting,
    connected,
    deferred,
    makePane('sftp', sftp),
  ];
  assert.equal(findReusableTerminalPane(panes, ssh), connecting);
  assert.equal(findReusableTerminalPane(panes, quick), connected);
  assert.equal(findReusableTerminalPane(panes, local), deferred);
  assert.equal(findReusableTerminalPane(panes, sftp), undefined);
  assert.equal(findReusableTerminalPane(panes.slice(0, 2), ssh), undefined);
});

test('closes every terminal and only file panes from other remotes', () => {
  const panes = [
    makePane('local', local),
    makePane('ssh', ssh),
    makePane('quick', quick, { exited }),
    makePane('saved-files', sftp),
    makePane('quick-files', { ...sftp, source: { address: quick.address } }),
    makePane('saved-editor', { ...editor, source: { hostId: ssh.hostId } }),
    makePane('quick-editor', editor),
    makePane('fleet', fleet),
  ];
  assert.deepEqual(panesToCloseForSingleTerminal(panes, ssh), ['local', 'ssh', 'quick', 'quick-files', 'quick-editor']);
  assert.deepEqual(panesToCloseForSingleTerminal(panes, quick), ['local', 'ssh', 'quick', 'saved-files', 'saved-editor']);
  assert.deepEqual(panesToCloseForSingleTerminal(panes, local), [
    'local', 'ssh', 'quick', 'saved-files', 'quick-files', 'saved-editor', 'quick-editor',
  ]);
});

test('keeps saved-host and quick-connect remote identities distinct', () => {
  const panes = [makePane('files', sftp)];
  assert.deepEqual(panesToCloseForSingleTerminal(panes, { kind: 'quick', address: ssh.hostId }), ['files']);
});

test('finds a background terminal by connection state, then creation order', () => {
  const live = makePane('live', ssh, { sessionId: 'session-1' });
  const newerLive = makePane('newer-live', quick, { sessionId: 'session-2' });
  const connecting = makePane('connecting', ssh);
  const newerConnecting = makePane('newer-connecting', local);
  const failed = makePane('failed', local, { failure });
  const ended = makePane('ended', ssh, { sessionId: 'session-3', exited });
  const deferred = makePane('deferred', quick, { deferred: true });
  const inactive = [failed, ended, makePane('fleet', fleet)];
  const pending = [connecting, newerConnecting, deferred, ...inactive];
  const all = [live, newerLive, ...pending];
  assert.equal(findBackgroundTerminal(all, null), newerLive);
  assert.equal(findBackgroundTerminal(all, newerLive.id), live);
  assert.equal(findBackgroundTerminal(pending, null), newerConnecting);
  assert.equal(findBackgroundTerminal(pending, newerConnecting.id), connecting);
  assert.equal(findBackgroundTerminal(inactive, null), ended);
  assert.equal(findBackgroundTerminal(inactive, ended.id), failed);
  assert.equal(findBackgroundTerminal([live, makePane('files', sftp)], live.id), undefined);
  assert.equal(findBackgroundTerminal([], null), undefined);
});
