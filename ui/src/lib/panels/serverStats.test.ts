import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { test } from 'node:test';

// Load the real helpers and defaults under Node's type stripping. The settings state only
// needs an identity initializer here, and these tests do not mount the app or invoke Tauri.
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
    if (!url.endsWith('/prefs.svelte.ts')) return result;
    return { ...result, source: `const $state = (value) => value;\n${result.source}` };
  },
});

const { cpuUsage, describeTailscalePath, formatLatencyMs, formatNetworkRate, networkRates } =
  await import('./serverStats.svelte.ts');
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
  ];
  const now = [
    { name: 'eth0', rx: 300, tx: 160 },
    { name: 'eth1', rx: 40, tx: 260 },
    { name: 'lo', rx: 500, tx: 500 },
    { name: 'new', rx: 999, tx: 999 },
  ];
  const { rates, totalRate } = networkRates(now, before, 2);
  assert.deepEqual({ ...rates }, { eth0: { rx: 100, tx: 30 }, eth1: { rx: 0, tx: 30 }, lo: { rx: 200, tx: 200 } });
  assert.deepEqual(totalRate, { rx: 100, tx: 60 });
  assert.equal(networkRates(now, before, 0).totalRate, null);
  assert.equal(networkRates(now, [], 2).totalRate, null);
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
    terminal_bar_network: true,
    terminal_bar_latency: true,
  });
});
