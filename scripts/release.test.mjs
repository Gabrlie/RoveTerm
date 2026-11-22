import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  buildAndroidManifest,
  buildGhReleaseCommand,
  buildLatestManifest,
  bumpWorkspaceVersion,
  collectArtifacts,
  extractChangelogSection,
  parseVersion,
  renderReleaseNotes,
} from './release.mjs';

function withTempDir(fn) {
  const dir = mkdtempSync(join(tmpdir(), 'roveterm-release-test-'));
  try {
    fn(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

describe('scripts/release.mjs', () => {
  it('validates SemVer strings and bumps Cargo.toml and package.json together', () => {
    assert.equal(parseVersion('v0.2.0'), '0.2.0');
    assert.equal(parseVersion('0.3.0-beta.1'), '0.3.0-beta.1');
    assert.throws(() => parseVersion('0.2'));
    assert.throws(() => parseVersion('latest'));

    withTempDir((dir) => {
      writeFileSync(
        join(dir, 'Cargo.toml'),
        '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n',
      );
      writeFileSync(
        join(dir, 'package.json'),
        '{\n  "name": "roveterm",\n  "version": "0.1.0",\n  "private": true\n}\n',
      );

      const res = bumpWorkspaceVersion(dir, 'v0.2.0');
      assert.equal(res.version, '0.2.0');
      assert.match(readFileSync(join(dir, 'Cargo.toml'), 'utf8'), /version = "0\.2\.0"/);
      assert.match(readFileSync(join(dir, 'package.json'), 'utf8'), /"version": "0\.2\.0"/);
    });
  });

  it('collects multi-platform artifacts and builds/merges latest.json and latest-android.json', () => {
    withTempDir((dir) => {
      const macDir = join(dir, 'macos');
      const winDir = join(dir, 'windows');
      const linuxDir = join(dir, 'linux');
      const androidDir = join(dir, 'android');
      for (const d of [macDir, winDir, linuxDir, androidDir]) mkdirSync(d, { recursive: true });

      writeFileSync(join(macDir, 'RoveTerm_0.2.0_aarch64.dmg'), 'dmg-arm64');
      writeFileSync(join(macDir, 'RoveTerm_0.2.0_x64.dmg'), 'dmg-x64');
      writeFileSync(join(macDir, 'RoveTerm_0.2.0_universal.dmg'), 'dmg-universal');
      writeFileSync(join(macDir, 'RoveTerm_0.2.0_universal.app.tar.gz'), 'tar-universal');
      writeFileSync(join(macDir, 'RoveTerm_0.2.0_universal.app.tar.gz.sig'), 'SIG-MAC-UNIVERSAL\n');

      writeFileSync(join(winDir, 'RoveTerm_0.2.0_x64-setup.exe'), 'win-nsis');
      writeFileSync(join(winDir, 'RoveTerm_0.2.0_x64-setup.exe.sig'), 'SIG-WIN-X64\n');
      writeFileSync(join(winDir, 'RoveTerm_0.2.0_arm64-setup.exe'), 'win-nsis-arm64');
      writeFileSync(join(winDir, 'RoveTerm_0.2.0_arm64-setup.exe.sig'), 'SIG-WIN-ARM64\n');

      writeFileSync(join(linuxDir, 'RoveTerm_0.2.0_amd64.AppImage'), 'linux-appimage');
      writeFileSync(join(linuxDir, 'RoveTerm_0.2.0_amd64.AppImage.sig'), 'SIG-LINUX-X64\n');
      writeFileSync(join(linuxDir, 'RoveTerm_0.2.0_aarch64.AppImage'), 'linux-appimage-arm64');
      writeFileSync(join(linuxDir, 'RoveTerm_0.2.0_aarch64.AppImage.sig'), 'SIG-LINUX-ARM64\n');
      writeFileSync(join(linuxDir, 'roveterm_0.2.0_amd64.deb'), 'linux-deb');
      writeFileSync(join(linuxDir, 'roveterm_0.2.0_arm64.deb'), 'linux-deb-arm64');
      writeFileSync(join(linuxDir, 'RoveTerm-0.2.0-1.aarch64.rpm'), 'linux-rpm-arm64');

      writeFileSync(join(androidDir, 'RoveTerm_0.2.0_arm64-v8a.apk'), 'android-apk');

      const collected = collectArtifacts([macDir, winDir, linuxDir, androidDir], {
        version: '0.2.0',
      });
      assert.equal(collected.installers.length, 11);
      assert.equal(collected.updaterPackages.length, 5);
      assert.equal(collected.androidApks.length, 1);

      const names = collected.installers.map((item) => item.name);
      assert.deepEqual(names.slice(0, 3), [
        'RoveTerm-0.2.0-macos-arm64.dmg',
        'RoveTerm-0.2.0-macos-x64.dmg',
        'RoveTerm-0.2.0-macos-universal.dmg',
      ]);
      assert.ok(names.includes('RoveTerm-0.2.0-windows-x64-setup.exe'));
      assert.ok(names.includes('RoveTerm-0.2.0-windows-arm64-setup.exe'));
      assert.ok(names.includes('RoveTerm-0.2.0-linux-x64.AppImage'));
      assert.ok(names.includes('RoveTerm-0.2.0-linux-arm64.AppImage'));
      assert.ok(names.includes('RoveTerm-0.2.0-linux-x64.deb'));
      assert.ok(names.includes('RoveTerm-0.2.0-linux-arm64.deb'));
      assert.ok(names.includes('RoveTerm-0.2.0-linux-arm64.rpm'));
      assert.ok(names.includes('RoveTerm-0.2.0-android-arm64.apk'));

      const latest = buildLatestManifest({
        version: '0.2.0',
        tag: 'v0.2.0',
        notes: 'Plain notes',
        pubDate: '2026-11-22T09:00:00Z',
        updaterPackages: collected.updaterPackages,
      });

      // Strictly Tauri v2 fields only (no unknown keys that would break tauri-plugin-updater).
      assert.deepEqual(Object.keys(latest).sort(), ['notes', 'platforms', 'pub_date', 'version']);
      assert.equal(latest.platforms['darwin-aarch64'].signature, 'SIG-MAC-UNIVERSAL');
      assert.match(latest.platforms['darwin-aarch64'].url, /RoveTerm-0\.2\.0-macos-universal\.app\.tar\.gz$/);
      assert.equal(latest.platforms['darwin-x86_64'].signature, 'SIG-MAC-UNIVERSAL');
      assert.equal(latest.platforms['windows-x86_64'].signature, 'SIG-WIN-X64');
      assert.match(latest.platforms['windows-x86_64'].url, /RoveTerm-0\.2\.0-windows-x64-setup\.exe$/);
      assert.equal(latest.platforms['windows-aarch64'].signature, 'SIG-WIN-ARM64');
      assert.match(latest.platforms['windows-aarch64'].url, /RoveTerm-0\.2\.0-windows-arm64-setup\.exe$/);
      assert.equal(latest.platforms['linux-x86_64'].signature, 'SIG-LINUX-X64');
      assert.match(latest.platforms['linux-x86_64'].url, /RoveTerm-0\.2\.0-linux-x64\.AppImage$/);
      assert.equal(latest.platforms['linux-aarch64'].signature, 'SIG-LINUX-ARM64');
      assert.match(latest.platforms['linux-aarch64'].url, /RoveTerm-0\.2\.0-linux-arm64\.AppImage$/);

      const android = buildAndroidManifest({
        version: '0.2.0',
        tag: 'v0.2.0',
        notes: 'Plain notes',
        pubDate: '2026-11-22T09:00:00Z',
        androidApks: collected.androidApks,
      });
      assert.equal(android.version, '0.2.0');
      assert.match(android.platforms['arm64-v8a'].url, /RoveTerm-0\.2\.0-android-arm64\.apk$/);
      assert.match(android.platforms['arm64-v8a'].sha256, /^[a-f0-9]{64}$/);
      assert.equal(android.platforms['arm64-v8a'].size, 'android-apk'.length);

      const notes = renderReleaseNotes({
        version: '0.2.0',
        tag: 'v0.2.0',
        previousTag: 'v0.1.0',
        changelogSection: '### 中文更新日志\n- 新特性\n\n### English Changelog\n- New feature',
        installers: collected.installers,
      });
      // No duplicate H1 title (GitHub Releases already renders --title), and changelog precedes downloads.
      assert.doesNotMatch(notes, /^# RoveTerm v/m);
      assert.ok(notes.indexOf('中文更新日志') < notes.indexOf('Download based on your OS'));
      assert.doesNotMatch(notes, /style=for-the-badge/);
      assert.match(notes, /RoveTerm-0\.2\.0-macos-arm64\.dmg/);
      assert.match(notes, /img\.shields\.io\/badge\/DMG-Apple%20Silicon-000000\.svg\?logo=apple/);
      assert.match(notes, /img\.shields\.io\/badge\/DMG-Intel%20x64-374151\.svg\?logo=apple&logoColor=white/);
      assert.match(notes, /alt="macOS Apple Silicon"[\s\S]*alt="macOS Intel x64"[\s\S]*alt="macOS Universal"/);
      assert.match(notes, /<\/table>\n<\/div>\n\n\*\*macOS 安装 \/ Installation\*\*/);
      assert.match(notes, /已损坏，无法打开[\s\S]*RoveTerm is damaged[\s\S]*```bash\nxattr -cr \/Applications\/RoveTerm\.app\n```\n\n<details>/);
      assert.match(notes, /img\.shields\.io\/badge\/Setup-x64-2d7d9a\.svg\?logo=windows/);
      assert.match(notes, /img\.shields\.io\/badge\/Setup-ARM64-67b7d1\.svg\?logo=windows/);
      assert.match(notes, /img\.shields\.io\/badge\/AppImage-ARM64-d94826\.svg\?logo=linux/);
      assert.match(notes, /img\.shields\.io\/badge\/DebPackage-ARM64-e07a5f\.svg\?logo=debian/);
      assert.match(notes, /img\.shields\.io\/badge\/RpmPackage-ARM64-d4961a\.svg\?logo=redhat/);
      assert.match(notes, /img\.shields\.io\/badge\/APK-ARMv8-168039\.svg\?logo=android/);
      assert.match(notes, /SHA-256 校验和/);
      assert.match(notes, /compare\/v0\.1\.0\.\.\.v0\.2\.0/);
    });
  });

  it('omits the macOS installation note when only other platforms have installers', () => {
    withTempDir((dir) => {
      writeFileSync(join(dir, 'RoveTerm_0.2.0_x64-setup.exe'), 'win-nsis');
      const { installers } = collectArtifacts([dir], { version: '0.2.0' });
      const notes = renderReleaseNotes({
        version: '0.2.0',
        tag: 'v0.2.0',
        changelogSection: '### English Changelog\n- New feature',
        installers,
      });
      assert.match(notes, /<td>Windows<\/td>/);
      assert.doesNotMatch(notes, /macOS 安装|xattr -cr/);
    });
  });

  it('extracts matching or Unreleased bilingual changelog sections and builds gh commands', () => {
    const sample = [
      '# Changelog',
      '',
      '## [Unreleased]',
      '',
      '### 中文',
      '- 新增应用内更新',
      '',
      '### English',
      '- Added in-app updater',
      '',
      '## [0.1.0] - 2026-11-01',
      '',
      '- Initial release',
    ].join('\n');

    assert.match(extractChangelogSection(sample, '0.1.0'), /Initial release/);
    assert.match(extractChangelogSection(sample, '0.2.0'), /Added in-app updater/);

    const ghArgs = buildGhReleaseCommand({
      tag: 'v0.2.0',
      title: 'RoveTerm v0.2.0',
      notesFile: 'dist/release/v0.2.0/RELEASE_NOTES.md',
      files: ['dist/release/v0.2.0/latest.json'],
      draft: true,
    });
    assert.deepEqual(ghArgs, [
      'release',
      'create',
      'v0.2.0',
      'dist/release/v0.2.0/latest.json',
      '--repo',
      'Gabrlie/RoveTerm',
      '--title',
      'RoveTerm v0.2.0',
      '--notes-file',
      'dist/release/v0.2.0/RELEASE_NOTES.md',
      '--draft',
    ]);
  });
});
