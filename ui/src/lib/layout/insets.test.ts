import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeVisualViewportIme, initInsets, sanitizeInsets, subscribeImeBottom, type MobileInsets } from './insets.ts';

const zero = { top: 0, right: 0, bottom: 0, left: 0, imeBottom: 0 };

test('defaults absent insets to zero', () => {
  assert.deepEqual(sanitizeInsets(null), zero);
  assert.deepEqual(sanitizeInsets(undefined), zero);
  assert.deepEqual(sanitizeInsets({ left: 12 }), { ...zero, left: 12 });
});

test('clamps invalid insets and rounds fractional CSS pixels', () => {
  assert.deepEqual(sanitizeInsets({ top: NaN, right: Infinity, bottom: -4, left: 13.4, imeBottom: 240.6 }), {
    ...zero,
    left: 13,
    imeBottom: 241,
  });
});

test('measures the gap below the visual viewport after accounting for its offset', () => {
  assert.equal(computeVisualViewportIme(800, 520.4, 20), 260);
  assert.equal(computeVisualViewportIme(800, 520, 0), 280);
});

test('returns zero for an already resized layout or an invalid viewport measurement', () => {
  assert.equal(computeVisualViewportIme(520, 520, 0), 0);
  assert.equal(computeVisualViewportIme(520, 521, 0), 0);
  assert.equal(computeVisualViewportIme(520, NaN, 0), 0);
  assert.equal(computeVisualViewportIme(Infinity, 520, 0), 0);
});

test('publishes effective IME insets from both the visual viewport and the native bridge', async (t) => {
  const visualViewport = Object.assign(new EventTarget(), { height: 800, offsetTop: 0 });
  const style = new Map<string, string>();
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { navigator: { userAgent: 'Android' }, innerHeight: 800, visualViewport },
  });
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: { documentElement: { style: { setProperty: style.set.bind(style) } } },
  });
  t.after(() => {
    Reflect.deleteProperty(globalThis, 'window');
    Reflect.deleteProperty(globalThis, 'document');
  });

  const updates: MobileInsets[] = [];
  const imeUpdates: number[] = [];
  t.after(subscribeImeBottom((value) => imeUpdates.push(value)));
  const native = { ...zero, bottom: 24, imeBottom: 300 };
  const stop = initInsets({
    fetch: async () => native,
    onChange: (insets) => updates.push(insets),
  });
  t.after(stop);

  visualViewport.height = 550;
  visualViewport.dispatchEvent(new Event('resize'));
  assert.equal(style.get('--ime-bottom'), '250px');
  assert.equal(style.has('--safe-bottom'), false);
  await Promise.resolve();

  assert.deepEqual(updates, [zero, { ...zero, imeBottom: 250 }, native]);
  assert.deepEqual(imeUpdates, [0, 0, 250, 300]);
  assert.equal(style.get('--ime-bottom'), '300px');
  assert.equal(style.get('--safe-bottom'), '24px');
});
