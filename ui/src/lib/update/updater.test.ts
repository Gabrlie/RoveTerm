import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  downloadPercent,
  effectiveUpdateChannel,
  formatReleaseDate,
  formatUpdateBytes,
  isPrereleaseVersion,
  isSkippedVersion,
  normalizeVersion,
  shouldAutoPromptUpdate,
  shouldShowUpdateBadge,
} from './updaterLogic.ts';

describe('updaterLogic', () => {
  it('normalizes version strings with or without leading v', () => {
    assert.equal(normalizeVersion('v0.2.0'), '0.2.0');
    assert.equal(normalizeVersion('  V1.0.0-rc.1 '), '1.0.0-rc.1');
    assert.equal(normalizeVersion(null), '');
  });

  it('detects prerelease versions and resolves effective update channels', () => {
    assert.equal(isPrereleaseVersion('0.2.1'), false);
    assert.equal(isPrereleaseVersion('0.2.1+build.1'), false);
    assert.equal(isPrereleaseVersion('v0.3.0-beta.1'), true);
    assert.equal(isPrereleaseVersion('0.3.0-rc.2+meta'), true);

    assert.equal(effectiveUpdateChannel(null, '0.2.1'), 'stable');
    assert.equal(effectiveUpdateChannel(undefined, '0.3.0-beta.1'), 'beta');
    assert.equal(effectiveUpdateChannel('beta', '0.2.1'), 'beta');
    assert.equal(effectiveUpdateChannel('stable', '0.3.0-beta.1'), 'stable');
  });

  it('tracks skipped version visibility for the status bar badge', () => {
    assert.equal(isSkippedVersion('0.2.0', 'v0.2.0'), true);
    assert.equal(isSkippedVersion('0.2.1', '0.2.0'), false);
    assert.equal(isSkippedVersion('0.2.0', ''), false);

    assert.equal(shouldShowUpdateBadge('available', '0.2.0', ''), true);
    assert.equal(shouldShowUpdateBadge('available', '0.2.0', '0.2.0'), false);
    assert.equal(shouldShowUpdateBadge('available', '0.2.1', '0.2.0'), true);
    assert.equal(shouldShowUpdateBadge('ready', '0.2.0', '0.2.0'), true);
    assert.equal(shouldShowUpdateBadge('downloading', '0.2.0', '0.2.0'), true);
    assert.equal(shouldShowUpdateBadge('up_to_date', '0.2.0', ''), false);
    assert.equal(shouldShowUpdateBadge('idle', null, ''), false);
  });

  it('computes bounded download percentages', () => {
    assert.equal(downloadPercent(0, 1000), 0);
    assert.equal(downloadPercent(500, 1000), 50);
    assert.equal(downloadPercent(1500, 1000), 100);
    assert.equal(downloadPercent(500, null), null);
    assert.equal(downloadPercent(500, 0), null);
  });

  it('formats download byte counts and ISO release dates', () => {
    assert.equal(formatUpdateBytes(0), '0 B');
    assert.equal(formatUpdateBytes(512), '512 B');
    assert.equal(formatUpdateBytes(1536), '1.5 KB');
    assert.equal(formatUpdateBytes(15 * 1024 * 1024), '15.0 MB');
    assert.equal(formatUpdateBytes(1.25 * 1024 * 1024 * 1024), '1.25 GB');

    assert.equal(formatReleaseDate('2026-11-22T08:00:00Z'), '2026-11-22');
    assert.equal(formatReleaseDate(null), '');
    assert.equal(formatReleaseDate('invalid'), '');
  });

  it('auto-prompts update dialog once per version on mobile unless skipped', () => {
    assert.equal(
      shouldAutoPromptUpdate({
        isMobile: true,
        available: true,
        version: '0.3.0-beta.3',
        skipped: '',
        lastPromptedVersion: null,
      }),
      true,
    );
    assert.equal(
      shouldAutoPromptUpdate({
        isMobile: false,
        available: true,
        version: '0.3.0-beta.3',
        skipped: '',
        lastPromptedVersion: null,
      }),
      false,
    );
    assert.equal(
      shouldAutoPromptUpdate({
        isMobile: true,
        available: true,
        version: 'v0.3.0-beta.3',
        skipped: '0.3.0-beta.3',
        lastPromptedVersion: null,
      }),
      false,
    );
    assert.equal(
      shouldAutoPromptUpdate({
        isMobile: true,
        available: true,
        version: '0.3.0-beta.3',
        skipped: '',
        lastPromptedVersion: '0.3.0-beta.3',
      }),
      false,
    );
    assert.equal(
      shouldAutoPromptUpdate({
        isMobile: true,
        available: true,
        version: '0.3.0-beta.4',
        skipped: '',
        lastPromptedVersion: '0.3.0-beta.3',
      }),
      true,
    );
  });
});
