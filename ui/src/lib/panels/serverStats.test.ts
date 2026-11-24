import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { test, type TestContext } from 'node:test';
import { setImmediate } from 'node:timers/promises';
import type { ServerStats } from '../ipc.ts';
import type { PaneState, PaneTarget } from '../workspace/workspace.svelte.ts';

// Shim Svelte runes for Node tests that use the real sampler and IPC wrapper.
const stubs: Record<string, string> = {
  svelte: 'export const untrack = (read) => read();',
  '../workspace/workspace.svelte': 'export const sourceKey = JSON.stringify;',
  './appearance': 'export const applyAppearance = () => {};',
};
const hooks = registerHooks({
  resolve(specifier, context, nextResolve) {
    const stub = stubs[specifier];
    if (stub) return { url: `data:text/javascript,${encodeURIComponent(stub)}`, shortCircuit: true };
    if (specifier === '../ipc' || specifier === '../settings/prefs.svelte') {
      return nextResolve(`${specifier}.ts`, context);
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    const result = nextLoad(url, context);
    if (!url.endsWith('/prefs.svelte.ts') && !url.endsWith('/serverStats.svelte.ts')) return result;
    return { ...result, source: `
      const $state = Object.assign((value) => value, { raw: (value) => value });
      const $effect = Object.assign((start) => start(), { root: (start) => { start(); return () => {}; } });
      ${result.source}
    ` };
  },
});

const {
  acquire, combinedNetworkRate, cpuUsage, describeTailscalePath, formatLatencyMs, formatMemoryBar,
  formatNetworkRate, hasOtherLivePaneForStats, invalidateServerStats, networkRates, paneStatsSource, statsSourceKey,
} = await import('./serverStats.svelte.ts');
const { prefs } = await import('../settings/prefs.svelte.ts');
hooks.deregister();

test('formats network rates in binary byte units or decimal bit units', () => {
  assert.equal(formatNetworkRate(undefined, 'bytes'), '…');
  assert.equal(formatNetworkRate(undefined, 'bits'), '…');
  assert.equal(formatNetworkRate(0, 'bytes'), '0 B/s');
  assert.equal(formatNetworkRate(512, 'bytes'), '512 B/s');
  assert.equal(formatNetworkRate(1024, 'bytes'), '1.0 KB/s');
  assert.equal(formatNetworkRate(1.5 * 1024 ** 2, 'bytes'), '1.5 MB/s');
  assert.equal(formatNetworkRate(2 * 1024 ** 3, 'bytes'), '2.0 GB/s');
  assert.equal(formatNetworkRate(100, 'bits'), '800 bps');
  assert.equal(formatNetworkRate(125, 'bits'), '1.0 Kbps');
  assert.equal(formatNetworkRate(200_000, 'bits'), '1.6 Mbps');
  assert.equal(formatNetworkRate(250_000_000, 'bits'), '2.0 Gbps');
});

test('formats sub-10ms latency with one decimal and preserves segment order', () => {
  assert.equal(formatLatencyMs([]), '');
  assert.equal(formatLatencyMs([2_550, 9_900, 10_000, 28_510]), '2.5+9.9+10+29');
});

test('describes direct, relayed, and unknown paths', () => {
  assert.deepEqual(describeTailscalePath({ kind: 'direct', endpoint: '192.0.2.1:41641' }),
    { kind: 'direct', title: '192.0.2.1:41641' });
  assert.deepEqual(describeTailscalePath({ kind: 'relay', region_id: 20, region_code: 'hkg', region_name: 'Hong Kong' }),
    { kind: 'relay', region: 'HKG', title: 'Hong Kong (HKG)' });
  assert.deepEqual(describeTailscalePath({ kind: 'relay', region_id: 7, region_code: '', region_name: 'Tokyo' }),
    { kind: 'relay', region: 'Tokyo', title: 'Tokyo' });
  assert.equal(describeTailscalePath({ kind: 'unknown' }), null);
  assert.equal(describeTailscalePath(null), null);
  assert.equal(describeTailscalePath(undefined), null);
});

test('calculates CPU activity from counter deltas and skips invalid samples', () => {
  assert.equal(cpuUsage({ total: 200, idle: 140 }, { total: 100, idle: 80 }), 40);
  assert.equal(cpuUsage({ total: 200, idle: 80 }, { total: 100, idle: 80 }), 100);
  assert.equal(cpuUsage({ total: 200, idle: 190 }, { total: 100, idle: 80 }), 0);
  assert.equal(cpuUsage({ total: 100, idle: 80 }, undefined), null);
  assert.equal(cpuUsage({ total: 100, idle: 80 }, { total: 100, idle: 80 }), null);
  assert.equal(cpuUsage({ total: 200, idle: 70 }, { total: 100, idle: 80 }), null);
});

test('calculates interface deltas, clamps resets, and totals only non-loopback matches', () => {
  const before = [
    { name: 'eth0', rx: 100, tx: 100 },
    { name: 'eth1', rx: 200, tx: 200 },
    { name: 'lo', rx: 100, tx: 100 },
    { name: 'lo0', rx: 100, tx: 100 },
  ];
  const now = [
    { name: 'eth0', rx: 300, tx: 160 },
    { name: 'eth1', rx: 40, tx: 260 },
    { name: 'lo', rx: 500, tx: 500 },
    { name: 'lo0', rx: 700, tx: 700 },
    { name: 'new', rx: 999, tx: 999 },
  ];
  const { rates, totalRate } = networkRates(now, before, 2);
  assert.deepEqual({ ...rates }, {
    eth0: { rx: 100, tx: 30 }, eth1: { rx: 0, tx: 30 }, lo: { rx: 200, tx: 200 }, lo0: { rx: 300, tx: 300 },
  });
  assert.deepEqual(totalRate, { rx: 100, tx: 60 });
  assert.equal(networkRates(now, before, 0).totalRate, null);
  assert.equal(networkRates(now, [], 2).totalRate, null);
});

type StatsPane = Pick<PaneState, 'id' | 'target' | 'sessionId' | 'exited' | 'failure' | 'deferred'>;
const remote = { hostId: 'saved' };
const quick = { address: 'user@example.test' };
const local = { kind: 'local' } as const;
const ssh = { kind: 'ssh', hostId: remote.hostId, title: 'Host' } as const;
const failure = { code: 'ssh.not_connected', message: 'Disconnected' };
const hostExists = (id: string) => id === remote.hostId;

function pane(target: PaneTarget, overrides: Partial<StatsPane> = {}): StatsPane {
  return { id: 'pane', target, sessionId: 'session', exited: null, failure: null, deferred: false, ...overrides };
}

test('only selects an eligible pane for status sampling', () => {
  assert.equal(paneStatsSource(undefined, hostExists), null);
  assert.equal(paneStatsSource(pane({ kind: 'fleet' }), hostExists), null);
  for (const overrides of [
    { deferred: true }, { sessionId: null }, { exited: { code: null, signal: null } }, { failure },
  ]) {
    assert.equal(paneStatsSource(pane(ssh, overrides), hostExists), null);
  }
  assert.equal(paneStatsSource(pane({ ...ssh, hostId: 'deleted' }), hostExists), null);
  assert.deepEqual(paneStatsSource(pane(ssh), hostExists), remote);
  assert.deepEqual(paneStatsSource(pane({ kind: 'quick', ...quick }), hostExists), quick);
  assert.equal(paneStatsSource(pane({ kind: 'quick', ...quick }, { sessionId: null }), hostExists), null);
  assert.deepEqual(paneStatsSource(pane(local), hostExists), local);
  assert.equal(paneStatsSource(pane(local, { sessionId: null }), hostExists), null);
  for (const target of [
    { kind: 'sftp', source: remote, title: 'Files' },
    { kind: 'editor', source: remote, title: 'Editor', draftId: 'draft', path: '/file' },
  ] as const) {
    assert.deepEqual(paneStatsSource(pane(target, { sessionId: null }), hostExists), remote);
    assert.deepEqual(paneStatsSource(pane({ ...target, source: quick }, { sessionId: null }), hostExists), quick);
    assert.equal(paneStatsSource(pane({ ...target, source: { hostId: 'deleted' } }), hostExists), null);
    assert.equal(paneStatsSource(pane(target, { failure }), hostExists), null);
  }
});

test('finds other live panes sharing a source and skips the pane being closed', () => {
  const panes = [
    pane(ssh),
    pane(ssh, { id: 'connecting', sessionId: null }),
    pane(ssh, { id: 'exited', exited: { code: 0, signal: null } }),
    pane(ssh, { id: 'deferred', deferred: true }),
    pane({ kind: 'sftp', source: remote, title: 'Files' }, { id: 'failed', sessionId: null, failure }),
    pane(local, { id: 'local' }),
    pane({ kind: 'quick', ...quick }, { id: 'quick' }),
  ];
  assert.equal(hasOtherLivePaneForStats(panes, 'pane', statsSourceKey(remote), hostExists), false);
  assert.equal(hasOtherLivePaneForStats(panes, 'local', statsSourceKey(local), hostExists), false);
  assert.equal(hasOtherLivePaneForStats(panes, 'pane', statsSourceKey(local), hostExists), true);
  assert.equal(hasOtherLivePaneForStats(panes, 'pane', statsSourceKey(quick), hostExists), true);
  panes.push(pane({ kind: 'sftp', source: remote, title: 'Files' }, { id: 'files', sessionId: null }));
  assert.equal(hasOtherLivePaneForStats(panes, 'pane', statsSourceKey(remote), hostExists), true);
  assert.equal(hasOtherLivePaneForStats(panes, 'pane', statsSourceKey(remote), () => false), false);
});

const sample: ServerStats = {
  load: [0, 0, 0], cpu: { total: 100, idle: 60 }, cpus: [{ total: 100, idle: 60 }], cores: 1,
  mem_total: 100, mem_available: 60, swap_total: 0, swap_free: 0, disks: [], uptime_secs: 120,
  interfaces: [{ name: 'eth0', rx: 20, tx: 30 }], kernel: 'Linux', hostname: 'test', os: 'Linux', latency_us: [],
};

function mockStatsIpc(t: TestContext) {
  const calls: { result: ReturnType<typeof Promise.withResolvers<ServerStats>>; args: unknown }[] = [];
  t.mock.timers.enable({ apis: ['setTimeout'] });
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      __TAURI_INTERNALS__: {
        invoke(command: string, args: unknown) {
          assert.equal(command, 'server_stats');
          const result = Promise.withResolvers<ServerStats>();
          calls.push({ result, args });
          return result.promise;
        },
      },
    },
  });
  t.after(() => { Reflect.deleteProperty(globalThis, 'window'); });
  return (args: unknown) => {
    t.mock.timers.tick(1);
    const call = calls.shift();
    assert.ok(call);
    assert.deepEqual(call.args, args);
    return call.result;
  };
}

test('retains stats across live tab switches and resets on SSH disconnect', async (t) => {
  const poll = mockStatsIpc(t);
  const args = { hostId: remote.hostId, address: null, local: false };
  const cpu = { total: 200, idle: 120 };
  const updated = { ...sample, cpu, cpus: [cpu] };
  const first = acquire(remote, 'session');
  t.after(first.release);
  const empty = first.state;
  poll(args).resolve(sample);
  await setImmediate();
  assert.equal(first.state.stats, sample);
  first.trigger();
  poll(args).resolve(updated);
  await setImmediate();
  assert.equal(first.state.cpu, 40);
  assert.deepEqual(first.state.coreHistory, [[40]]);
  first.release();

  const next = acquire(remote, 'session');
  t.after(next.release);
  assert.equal(next.state.stats, updated);
  const unsupported = { code: 'monitor.unsupported', message: 'Unsupported' };
  poll(args).reject(unsupported);
  await setImmediate();
  assert.equal(next.state.stats, updated);
  assert.equal(next.state.problem, unsupported.code);
  assert.equal(next.state.error, unsupported);

  next.trigger();
  poll(args).reject(failure);
  await setImmediate();
  assert.deepEqual(next.state, empty);

  next.trigger();
  poll(args).resolve(updated);
  await setImmediate();
  assert.notEqual(next.state.stats, null);
  assert.equal(next.state.cpu, null);
  assert.deepEqual(next.state.coreHistory, [[]]);
  assert.equal(next.state.totalRate, null);
});

test('invalidation clears local stats and discards a sample already in flight', async (t) => {
  const poll = mockStatsIpc(t);
  const args = { hostId: null, address: null, local: true };
  const first = acquire(local, 'session');
  t.after(first.release);
  const empty = first.state;
  poll(args).resolve(sample);
  await setImmediate();
  assert.equal(first.state.stats, sample);

  first.trigger();
  const pending = poll(args);
  first.release();
  invalidateServerStats(local);
  const reopened = acquire(local, 'new-session');
  t.after(reopened.release);
  assert.deepEqual(reopened.state, empty);
  pending.resolve(sample);
  await setImmediate();
  assert.deepEqual(reopened.state, empty);

  poll(args).resolve(sample);
  await setImmediate();
  assert.equal(reopened.state.stats, sample);
});

test('formats terminal status bar memory in all display modes and combines network rates', () => {
  const fmt = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  const used = 4.2 * 1024 ** 3;
  const total = 16 * 1024 ** 3;
  assert.equal(formatMemoryBar(used, total, 'full_percent', fmt), '4.2 GB / 16.0 GB (26%)');
  assert.equal(formatMemoryBar(used, total, 'full', fmt), '4.2 GB / 16.0 GB');
  assert.equal(formatMemoryBar(used, total, 'used', fmt), '4.2 GB');
  assert.equal(formatMemoryBar(used, total, 'percent', fmt), '26%');
  assert.equal(formatMemoryBar(0, 0, 'percent', fmt), '0%');
  assert.equal(combinedNetworkRate(null), undefined);
  assert.equal(combinedNetworkRate({ rx: 1200, tx: 800 }), 2000);
});

test('uses the backend defaults before saved status preferences load', () => {
  assert.deepEqual(prefs.value.status, {
    interval_secs: 3,
    show_system: true,
    show_cpu: true,
    show_memory: true,
    show_swap: true,
    show_network: true,
    show_disks: true,
    cpu_display: 'auto',
    network_unit: 'bytes',
    terminal_bar: true,
    terminal_bar_cpu: true,
    terminal_bar_memory: true,
    terminal_bar_memory_display: 'full_percent',
    terminal_bar_network: true,
    terminal_bar_network_display: 'split',
    terminal_bar_latency: false,
    terminal_bar_tailscale_path: true,
  });
});
