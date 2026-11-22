import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  downloadPercent,
  formatReleaseDate,
  formatUpdateBytes,
  isSkippedVersion,
  normalizeVersion,
  shouldShowUpdateBadge,
} from './updaterLogic.ts';

describe('updaterLogic', () => {
  it('normalizes version strings with or without leading v', () => {
    assert.equal(normalizeVersion('v0.2.0'), '0.2.0');
    assert.equal(normalizeVersion('  V1.0.0-rc.1 '), '1.0.0-rc.1');
    assert.equal(normalizeVersion(null), '');
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
});
