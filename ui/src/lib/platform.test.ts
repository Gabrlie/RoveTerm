import { test } from 'node:test';
import assert from 'node:assert/strict';
import { capabilitiesFor, detectPlatform, type PlatformInfo } from './platform.ts';

const mac: PlatformInfo = {
  isAndroid: false,
  isIos: false,
  isMobile: false,
  isApple: true,
  isMacDesktop: true,
};
const ios: PlatformInfo = {
  isAndroid: false,
  isIos: true,
  isMobile: true,
  isApple: true,
  isMacDesktop: false,
};
const otherDesktop: PlatformInfo = {
  isAndroid: false,
  isIos: false,
  isMobile: false,
  isApple: false,
  isMacDesktop: false,
};
const macUserAgent = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';

test('detects macOS desktop with the Mac platform or user agent', () => {
  assert.deepEqual(detectPlatform({ platform: 'MacIntel', maxTouchPoints: 1 }), mac);
  assert.deepEqual(detectPlatform({ userAgent: macUserAgent }), mac);
});

test('detects iPadOS with a desktop user agent and multi-touch', () => {
  assert.deepEqual(detectPlatform({ userAgent: macUserAgent, platform: 'MacIntel', maxTouchPoints: 5 }), ios);
});

test('detects iPhone, iPad, and iPod user agents', () => {
  for (const device of ['iPhone', 'iPad', 'iPod']) {
    assert.deepEqual(detectPlatform({ userAgent: `Mozilla/5.0 (${device}; CPU OS 18_0 like Mac OS X)` }), ios);
  }
});

test('detects Android even when the platform reports Linux', () => {
  assert.deepEqual(detectPlatform({ userAgent: 'Mozilla/5.0 (Linux; Android 15)', platform: 'Linux aarch64' }), {
    ...otherDesktop,
    isAndroid: true,
    isMobile: true,
  });
});

test('detects Windows and Linux as desktop even with touch input', () => {
  assert.deepEqual(detectPlatform({ userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)', platform: 'Win32', maxTouchPoints: 10 }), otherDesktop);
  assert.deepEqual(detectPlatform({ userAgent: 'Mozilla/5.0 (X11; Linux x86_64)', platform: 'Linux x86_64' }), otherDesktop);
});

test('uses desktop defaults when platform information is absent', () => {
  assert.deepEqual(detectPlatform(), otherDesktop);
});

test('maps desktop and mobile features to platform capabilities', () => {
  assert.deepEqual(capabilitiesFor(mac), {
    localPty: true,
    schedules: true,
    localAgents: true,
    externalEditor: true,
    autostart: true,
    biometricVault: false,
  });
  assert.deepEqual(capabilitiesFor(ios), {
    localPty: false,
    schedules: false,
    localAgents: false,
    externalEditor: false,
    autostart: false,
    biometricVault: true,
  });
});
