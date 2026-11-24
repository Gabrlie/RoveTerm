import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isActionAvailable } from './actions.ts';
import { capabilitiesFor, detectPlatform } from './platform.ts';

const desktop = capabilitiesFor(detectPlatform({ platform: 'MacIntel' }));
const mobile = capabilitiesFor(detectPlatform({ userAgent: 'iPhone' }));

test('local terminals and schedules require desktop capabilities', () => {
  for (const id of ['tab.newLocal', 'fleet.schedules'] as const) {
    assert.equal(isActionAvailable(id, desktop), true);
    assert.equal(isActionAvailable(id, mobile), false);
  }
});

test('local terminal and schedule availability follow independent capabilities', () => {
  const localOnly = { ...mobile, localPty: true };
  const schedulesOnly = { ...mobile, schedules: true };
  assert.equal(isActionAvailable('tab.newLocal', localOnly), true);
  assert.equal(isActionAvailable('fleet.schedules', localOnly), false);
  assert.equal(isActionAvailable('tab.newLocal', schedulesOnly), false);
  assert.equal(isActionAvailable('fleet.schedules', schedulesOnly), true);
});

test('quick connect and Fleet remain available on mobile', () => {
  for (const caps of [desktop, mobile]) {
    assert.equal(isActionAvailable('host.quickConnect', caps), true);
    assert.equal(isActionAvailable('fleet.open', caps), true);
  }
});
