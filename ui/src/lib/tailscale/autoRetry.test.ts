import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { HostEntry } from '../ipc';
import {
  shouldAutoRetryTailscale,
  showPasswordFallbackAction,
  showTailscaleSettingsAction,
  usesTailscaleRoute,
} from './autoRetry.ts';

function host(id: string, route: HostEntry['route']): HostEntry {
  return {
    id,
    name: id,
    group: '',
    hostname: `${id}.ts.net`,
    port: 22,
    username: 'root',
    auth: { method: 'agent' },
    route,
    tags: [],
    has_secret: false,
    os: null,
    created_at: 0,
    updated_at: 0,
    last_connected_at: null,
  };
}

test('detects direct and jump-chained Tailscale routes without looping on cycles', () => {
  const tsJump = host('jump', { kind: 'tailscale', peer_id: 'peer-1' });
  const behindJump = host('target', { kind: 'jump', host: 'jump' });
  const direct = host('direct', { kind: 'direct' });
  const cycleA = host('a', { kind: 'jump', host: 'b' });
  const cycleB = host('b', { kind: 'jump', host: 'a' });
  const byId = new Map([tsJump, behindJump, direct, cycleA, cycleB].map((item) => [item.id, item]));
  const find = (id: string) => byId.get(id);

  assert.equal(usesTailscaleRoute(tsJump, find), true);
  assert.equal(usesTailscaleRoute(behindJump, find), true);
  assert.equal(usesTailscaleRoute(direct, find), false);
  assert.equal(usesTailscaleRoute(cycleA, find), false);
  assert.equal(usesTailscaleRoute(null, find), false);
});

test('auto-retries failed Tailscale tabs when Tailscale connects and once on startup races', () => {
  assert.equal(shouldAutoRetryTailscale({
    isTailscaleHost: true,
    failureCode: 'tailscale.not_connected',
    connected: true,
    wasConnected: false,
    alreadyRetriedForEpoch: false,
  }), true);

  assert.equal(shouldAutoRetryTailscale({
    isTailscaleHost: true,
    failureCode: 'ssh.connect',
    connected: true,
    wasConnected: false,
    alreadyRetriedForEpoch: false,
  }), true);

  // Startup race: Tailscale finished connecting just before the not_connected error returned.
  assert.equal(shouldAutoRetryTailscale({
    isTailscaleHost: true,
    failureCode: 'tailscale.not_connected',
    connected: true,
    wasConnected: true,
    alreadyRetriedForEpoch: false,
  }), true);

  // Guard prevents looping within the same Tailscale connection epoch.
  assert.equal(shouldAutoRetryTailscale({
    isTailscaleHost: true,
    failureCode: 'tailscale.not_connected',
    connected: true,
    wasConnected: true,
    alreadyRetriedForEpoch: true,
  }), false);

  // Non-Tailscale host errors and SSH auth errors are not auto-retried.
  assert.equal(shouldAutoRetryTailscale({
    isTailscaleHost: false,
    failureCode: 'ssh.connect',
    connected: true,
    wasConnected: false,
    alreadyRetriedForEpoch: false,
  }), false);

  assert.equal(shouldAutoRetryTailscale({
    isTailscaleHost: true,
    failureCode: 'ssh.auth_failed',
    connected: true,
    wasConnected: false,
    alreadyRetriedForEpoch: false,
  }), false);
});

test('replaces password fallback with Tailscale settings on Tailscale failures', () => {
  assert.equal(showTailscaleSettingsAction({
    isTailscaleHost: true,
    tailscaleConnected: false,
    failureCode: 'tailscale.not_connected',
  }), true);
  assert.equal(showPasswordFallbackAction({
    isRemote: true,
    isTailscaleHost: true,
    tailscaleConnected: false,
    failureCode: 'tailscale.not_connected',
  }), false);

  // SSH auth failure after Tailscale is up still allows password sign-in.
  assert.equal(showTailscaleSettingsAction({
    isTailscaleHost: true,
    tailscaleConnected: true,
    failureCode: 'ssh.auth_failed',
  }), false);
  assert.equal(showPasswordFallbackAction({
    isRemote: true,
    isTailscaleHost: true,
    tailscaleConnected: true,
    failureCode: 'ssh.auth_failed',
  }), true);

  // Tailscale connect/timeout failure offers Tailscale settings, not password fallback.
  assert.equal(showTailscaleSettingsAction({
    isTailscaleHost: true,
    tailscaleConnected: true,
    failureCode: 'ssh.connect',
  }), true);
  assert.equal(showPasswordFallbackAction({
    isRemote: true,
    isTailscaleHost: true,
    tailscaleConnected: true,
    failureCode: 'ssh.connect',
  }), false);
});
