import assert from 'node:assert/strict';
import { test } from 'node:test';
import { siDebian, siProxmox } from 'simple-icons';
import { osIcon } from './osIcons.ts';

test('uses the Proxmox VE mark for both Proxmox identifiers', () => {
  for (const id of ['proxmox', 'pve']) {
    const icon = osIcon({ id, name: 'Proxmox VE', like: ['debian'] });
    assert.equal(icon?.path, siProxmox.path);
    assert.equal(icon?.color, `#${siProxmox.hex}`);
  }
});

test('uses the blue NAS glyph for fnOS and its trim alias', () => {
  const fnos = osIcon({ id: 'fnos', name: 'fnOS', like: ['debian'] });
  assert.ok(fnos);
  assert.equal(fnos.color, '#1677ff');
  assert.ok(fnos.path.startsWith('M'));
  assert.notEqual(fnos.path, siDebian.path);
  assert.deepEqual(osIcon({ id: 'trim', name: 'fnOS', like: ['debian'] }), fnos);
});

test('falls back to the first recognized parent distribution', () => {
  assert.equal(osIcon({ id: 'unknown', name: '', like: ['unknown', 'debian'] })?.path, siDebian.path);
});

test('returns null for missing or unknown OS information', () => {
  assert.equal(osIcon(null), null);
  assert.equal(osIcon({ id: 'unknown', name: '', like: [] }), null);
  assert.equal(osIcon({ id: 'constructor', name: '', like: [] }), null);
});
