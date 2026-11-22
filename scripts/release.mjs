#!/usr/bin/env node
// Local manual build & GitHub Release helper (replaces GitHub Actions).
// Manages synchronized workspace versions, Tauri v2 signed latest.json manifests,
// Android latest-android.json manifests, SHA-256 checksums and FlClash-style
// bilingual Release Notes for `gh release create / upload`.

import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { basename, dirname, extname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const DEFAULT_REPO = 'Gabrlie/RoveTerm';

const SEMVER_RE =
  /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

const DEFAULT_SCAN_DIRS = [
  'target/release/bundle',
  'target/universal-apple-darwin/release/bundle',
  'target/aarch64-apple-darwin/release/bundle',
  'target/x86_64-apple-darwin/release/bundle',
  'target/x86_64-pc-windows-msvc/release/bundle',
  'target/aarch64-pc-windows-msvc/release/bundle',
  'src-tauri/gen/android/app/build/outputs/apk',
];

/** Validates and normalizes a SemVer 2.0 string (strips an optional leading `v`). */
export function parseVersion(raw) {
  if (typeof raw !== 'string') {
    throw new Error('版本号不能为空 / Version is required.');
  }
  const trimmed = raw.trim();
  const match = SEMVER_RE.exec(trimmed);
  if (!match) {
    throw new Error(
      `非法版本号 "${trimmed}"：须符合 SemVer 2.0（例如 0.2.0 或 0.3.0-beta.1）。`,
    );
  }
  const [, major, minor, patch, pre, build] = match;
  return `${major}.${minor}.${patch}${pre ? `-${pre}` : ''}${build ? `+${build}` : ''}`;
}

/** Reads the workspace version from root Cargo.toml. */
export function readWorkspaceVersion(rootDir) {
  const cargoPath = join(rootDir, 'Cargo.toml');
  const text = readFileSync(cargoPath, 'utf8');
  const match = /^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(text);
  if (!match) {
    throw new Error(`无法从 ${cargoPath} 读取 [workspace.package].version`);
  }
  return parseVersion(match[1]);
}

/** Updates `[workspace.package].version` in Cargo.toml and `"version"` in package.json. */
export function bumpWorkspaceVersion(rootDir, rawVersion) {
  const version = parseVersion(rawVersion);
  const cargoPath = join(rootDir, 'Cargo.toml');
  const pkgPath = join(rootDir, 'package.json');

  const cargoText = readFileSync(cargoPath, 'utf8');
  let cargoUpdated = false;
  const nextCargo = cargoText.replace(
    /(^\[workspace\.package\][\s\S]*?^version\s*=\s*")([^"]+)(")/m,
    (_, prefix, _old, suffix) => {
      cargoUpdated = true;
      return `${prefix}${version}${suffix}`;
    },
  );
  if (!cargoUpdated) {
    throw new Error(`未在 ${cargoPath} 找到 [workspace.package].version`);
  }

  const pkgText = readFileSync(pkgPath, 'utf8');
  let pkgUpdated = false;
  const nextPkg = pkgText.replace(
    /(^\s*"version"\s*:\s*")([^"]+)(")/m,
    (_, prefix, _old, suffix) => {
      pkgUpdated = true;
      return `${prefix}${version}${suffix}`;
    },
  );
  if (!pkgUpdated) {
    throw new Error(`未在 ${pkgPath} 找到顶层 "version" 字段`);
  }

  writeFileSync(cargoPath, nextCargo, 'utf8');
  writeFileSync(pkgPath, nextPkg, 'utf8');
  return { version, updatedFiles: [cargoPath, pkgPath] };
}

/** Computes lowercase SHA-256 hex digest of a file. */
export function sha256File(filePath) {
  const hash = createHash('sha256');
  hash.update(readFileSync(filePath));
  return hash.digest('hex');
}

/** Formats bytes cleanly for release tables and badges. */
export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb < 100 ? mb.toFixed(1) : Math.round(mb)} MB`;
  return `${(mb / 1024).toFixed(2)} GB`;
}

function walkFiles(dir, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      // Skip macOS .app bundle internals; only top-level archives/installers belong in releases.
      if (entry.name.endsWith('.app')) continue;
      walkFiles(full, out);
    } else if (entry.isFile()) {
      out.push(full);
    }
  }
  return out;
}

function detectArch(text, fallback = 'x86_64') {
  const lower = text.toLowerCase();
  if (lower.includes('universal')) return 'universal';
  if (lower.includes('arm64-v8a')) return 'arm64-v8a';
  if (/(?:armeabi-v7a|armv7)/.test(lower)) return 'armeabi-v7a';
  if (/(?:aarch64|arm64)/.test(lower)) return 'aarch64';
  if (/(?:x86_64|amd64|x64)/.test(lower)) return 'x86_64';
  return fallback;
}

/** Formats an OS- and architecture-tagged release asset filename (e.g. RoveTerm-0.1.0-linux-arm64.deb). */
export function canonicalArtifactName(classified, version) {
  if (!version) return classified.name;
  const normalized = parseVersion(version);
  const archSlug =
    classified.os === 'android'
      ? classified.abi === 'arm64-v8a'
        ? 'arm64'
        : classified.abi === 'armeabi-v7a'
          ? 'armv7'
          : classified.abi === 'x86_64'
            ? 'x64'
            : 'universal'
      : classified.arch === 'universal'
        ? 'universal'
        : classified.arch === 'aarch64'
          ? 'arm64'
          : 'x64';
  const base = `RoveTerm-${normalized}-${classified.os}-${archSlug}`;
  switch (classified.format) {
    case 'exe':
      return `${base}-setup.exe`;
    case 'msi':
      return `${base}.msi`;
    case 'dmg':
      return `${base}.dmg`;
    case 'app.tar.gz':
      return `${base}.app.tar.gz`;
    case 'AppImage':
      return `${base}.AppImage`;
    case 'AppImage.tar.gz':
      return `${base}.AppImage.tar.gz`;
    case 'deb':
      return `${base}.deb`;
    case 'rpm':
      return `${base}.rpm`;
    case 'apk':
      return `${base}.apk`;
    default:
      return classified.name;
  }
}

/** Classifies a single file path into installer / desktop updater / android APK metadata. */
export function classifyArtifact(filePath) {
  const name = basename(filePath);
  const lower = name.toLowerCase();
  const context = filePath.toLowerCase();

  if (lower.endsWith('.sig')) return null;

  if (lower.endsWith('.app.tar.gz')) {
    const arch = detectArch(`${name} ${context}`, 'aarch64');
    const targets =
      arch === 'universal'
        ? ['darwin-aarch64', 'darwin-x86_64']
        : [`darwin-${arch === 'aarch64' ? 'aarch64' : 'x86_64'}`];
    return {
      role: 'updater',
      os: 'macos',
      arch,
      targets,
      format: 'app.tar.gz',
      name,
      path: filePath,
    };
  }

  if (lower.endsWith('.dmg')) {
    const arch = detectArch(`${name} ${context}`, 'aarch64');
    const label =
      arch === 'universal'
        ? 'Universal'
        : arch === 'aarch64'
          ? 'Apple Silicon'
          : 'Intel x64';
    const badgeColor =
      arch === 'universal' ? '00A9E0' : arch === 'aarch64' ? '000000' : '374151';
    return {
      role: 'installer',
      os: 'macos',
      arch,
      format: 'dmg',
      label,
      badgeLabel: 'DMG',
      badgeMessage: label,
      badgeColor,
      badgeLogo: 'apple',
      name,
      path: filePath,
    };
  }

  if (lower.endsWith('-setup.exe') || lower.endsWith('.exe')) {
    const arch = detectArch(`${name} ${context}`, 'x86_64');
    const archLabel = arch === 'aarch64' ? 'ARM64' : 'x64';
    const target = arch === 'aarch64' ? 'windows-aarch64' : 'windows-x86_64';
    return {
      role: 'both',
      os: 'windows',
      arch,
      targets: [target],
      format: 'exe',
      label: `${archLabel} NSIS`,
      badgeLabel: 'Setup',
      badgeMessage: archLabel,
      badgeColor: arch === 'aarch64' ? '67b7d1' : '2d7d9a',
      badgeLogo: 'windows',
      name,
      path: filePath,
    };
  }

  if (lower.endsWith('.msi')) {
    const arch = detectArch(`${name} ${context}`, 'x86_64');
    const archLabel = arch === 'aarch64' ? 'ARM64' : 'x64';
    const target = arch === 'aarch64' ? 'windows-aarch64' : 'windows-x86_64';
    return {
      role: 'both',
      os: 'windows',
      arch,
      targets: [target],
      format: 'msi',
      label: `${archLabel} MSI`,
      badgeLabel: 'MSI',
      badgeMessage: archLabel,
      badgeColor: arch === 'aarch64' ? '4ba3e3' : '0078D4',
      badgeLogo: 'windows',
      name,
      path: filePath,
    };
  }

  if (lower.endsWith('.appimage.tar.gz')) {
    const arch = detectArch(`${name} ${context}`, 'x86_64');
    return {
      role: 'updater',
      os: 'linux',
      arch,
      targets: [`linux-${arch === 'aarch64' ? 'aarch64' : 'x86_64'}`],
      format: 'AppImage.tar.gz',
      name,
      path: filePath,
    };
  }

  if (lower.endsWith('.appimage')) {
    const arch = detectArch(`${name} ${context}`, 'x86_64');
    const archLabel = arch === 'aarch64' ? 'ARM64' : 'x64';
    return {
      role: 'both',
      os: 'linux',
      arch,
      targets: [`linux-${arch === 'aarch64' ? 'aarch64' : 'x86_64'}`],
      format: 'AppImage',
      label: `${archLabel} AppImage`,
      badgeLabel: 'AppImage',
      badgeMessage: archLabel,
      badgeColor: arch === 'aarch64' ? 'd94826' : 'f84e29',
      badgeLogo: 'linux',
      name,
      path: filePath,
    };
  }

  if (lower.endsWith('.deb') || lower.endsWith('.rpm')) {
    const arch = detectArch(`${name} ${context}`, 'x86_64');
    const archLabel = arch === 'aarch64' ? 'ARM64' : 'x64';
    const ext = extname(name).slice(1).toLowerCase();
    const isDeb = ext === 'deb';
    return {
      role: 'installer',
      os: 'linux',
      arch,
      format: ext,
      label: `${archLabel} .${ext}`,
      badgeLabel: isDeb ? 'DebPackage' : 'RpmPackage',
      badgeMessage: archLabel,
      badgeColor: isDeb
        ? arch === 'aarch64'
          ? 'e07a5f'
          : 'FF9966'
        : arch === 'aarch64'
          ? 'd4961a'
          : 'F1B42F',
      badgeLogo: isDeb ? 'debian' : 'redhat',
      name,
      path: filePath,
    };
  }

  if (lower.endsWith('.apk')) {
    const rawArch = detectArch(`${name} ${context}`, 'universal');
    const abi =
      rawArch === 'aarch64' || rawArch === 'arm64-v8a'
        ? 'arm64-v8a'
        : rawArch === 'armeabi-v7a'
          ? 'armeabi-v7a'
          : rawArch === 'x86_64'
            ? 'x86_64'
            : 'universal';
    const badgeMessage =
      abi === 'arm64-v8a'
        ? 'ARMv8'
        : abi === 'armeabi-v7a'
          ? 'ARMv7'
          : abi === 'x86_64'
            ? 'x64'
            : 'Universal';
    const badgeColor =
      abi === 'arm64-v8a' || abi === 'universal'
        ? '168039'
        : abi === 'armeabi-v7a'
          ? '45bf55'
          : '96ed89';
    return {
      role: 'android',
      os: 'android',
      arch: abi,
      abi,
      format: 'apk',
      label: abi === 'universal' ? 'Universal APK' : `${abi} APK`,
      badgeLabel: 'APK',
      badgeMessage,
      badgeColor,
      badgeLogo: 'android',
      name,
      path: filePath,
    };
  }

  return null;
}

/** Scans directories for installers, signed updater packages, and Android APKs. */
export function collectArtifacts(dirs, { version = null } = {}) {
  const seenNames = new Set();
  const installers = [];
  const updaterPackages = [];
  const androidApks = [];
  const uploadFiles = new Map();

  for (const dir of dirs) {
    if (!existsSync(dir)) continue;
    for (const filePath of walkFiles(dir).sort()) {
      const classified = classifyArtifact(filePath);
      if (!classified) continue;
      const releaseName = canonicalArtifactName(classified, version);
      if (seenNames.has(releaseName)) continue;
      seenNames.add(releaseName);

      const stat = statSync(filePath);
      const sigPath = `${filePath}.sig`;
      const hasSig = existsSync(sigPath);
      const signature = hasSig ? readFileSync(sigPath, 'utf8').trim() : null;
      const item = {
        ...classified,
        sourceName: classified.name,
        name: releaseName,
        size: stat.size,
        sha256: sha256File(filePath),
        sigPath: hasSig ? sigPath : null,
        signature,
      };

      uploadFiles.set(item.name, filePath);
      if (hasSig) {
        uploadFiles.set(`${item.name}.sig`, sigPath);
      }

      if (item.role === 'installer' || item.role === 'both' || item.role === 'android') {
        installers.push(item);
      }
      if ((item.role === 'updater' || item.role === 'both') && item.signature) {
        updaterPackages.push(item);
      }
      if (item.role === 'android') {
        androidApks.push(item);
      }
    }
  }

  const osOrder = { macos: 0, windows: 1, linux: 2, android: 3 };
  const formatOrder = { dmg: 0, exe: 0, msi: 1, AppImage: 0, deb: 1, rpm: 2, apk: 0 };
  const archOrder = {
    x86_64: 0,
    aarch64: 1,
    universal: 2,
    'arm64-v8a': 0,
    'armeabi-v7a': 1,
  };
  const macArchOrder = { aarch64: 0, universal: 1, x86_64: 2 };
  installers.sort((a, b) => {
    const byOs = (osOrder[a.os] ?? 9) - (osOrder[b.os] ?? 9);
    if (byOs !== 0) return byOs;
    const byFmt = (formatOrder[a.format] ?? 9) - (formatOrder[b.format] ?? 9);
    if (byFmt !== 0) return byFmt;
    const map = a.os === 'macos' ? macArchOrder : archOrder;
    const byArch = (map[a.arch] ?? 9) - (map[b.arch] ?? 9);
    return byArch !== 0 ? byArch : a.name.localeCompare(b.name);
  });

  const uploadEntries = [...uploadFiles.entries()]
    .map(([name, srcPath]) => ({ name, srcPath }))
    .sort((a, b) => a.name.localeCompare(b.name));

  return {
    installers,
    updaterPackages,
    androidApks,
    uploadEntries,
    uploadFiles: uploadEntries.map((entry) => entry.srcPath),
  };
}

/** Extracts the Markdown body for `version` (or `Unreleased`) from CHANGELOG.md. */
export function extractChangelogSection(changelogText, version) {
  const normalized = parseVersion(version);
  const headings = [...changelogText.matchAll(/^## \[([^\]]+)\].*$/gm)];
  const findByKeys = (keys) => {
    for (let i = 0; i < headings.length; i += 1) {
      const title = headings[i][1].trim().replace(/^v/i, '');
      if (keys.includes(title.toLowerCase())) {
        const start = headings[i].index + headings[i][0].length;
        const end = i + 1 < headings.length ? headings[i + 1].index : changelogText.length;
        return changelogText.slice(start, end).trim();
      }
    }
    return '';
  };

  const exact = findByKeys([normalized.toLowerCase()]);
  if (exact) return exact;
  const unreleased = findByKeys(['unreleased']);
  if (unreleased) return unreleased;

  return [
    '### 中文更新日志',
    '',
    `- 发布 RoveTerm v${normalized}。`,
    '',
    '### English Changelog',
    '',
    `- Release RoveTerm v${normalized}.`,
  ].join('\n');
}

/** Builds a Tauri v2 compliant `latest.json` manifest (desktop targets only). */
export function buildLatestManifest({
  version,
  tag,
  repo = DEFAULT_REPO,
  notes,
  pubDate,
  updaterPackages,
  existing = null,
}) {
  const normalized = parseVersion(version);
  const platforms = { ...(existing?.platforms ?? {}) };
  for (const pkg of updaterPackages) {
    const url = `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(pkg.name)}`;
    for (const target of pkg.targets ?? []) {
      platforms[target] = {
        signature: pkg.signature,
        url,
      };
    }
  }
  return {
    version: normalized,
    notes: notes.trim(),
    pub_date: pubDate,
    platforms,
  };
}

/** Builds the standalone `latest-android.json` manifest for Android APK discovery & checksums. */
export function buildAndroidManifest({
  version,
  tag,
  repo = DEFAULT_REPO,
  notes,
  pubDate,
  androidApks,
  existing = null,
}) {
  const normalized = parseVersion(version);
  const platforms = { ...(existing?.platforms ?? {}) };
  for (const apk of androidApks) {
    const url = `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(apk.name)}`;
    platforms[apk.abi] = {
      url,
      sha256: apk.sha256,
      size: apk.size,
    };
  }
  return {
    version: normalized,
    notes: notes.trim(),
    pub_date: pubDate,
    release_url: `https://github.com/${repo}/releases/tag/${tag}`,
    platforms,
  };
}

const OS_META = {
  macos: { title: 'macOS', badgeLabel: 'DMG', color: '000000', logo: 'apple' },
  windows: { title: 'Windows', badgeLabel: 'Setup', color: '2d7d9a', logo: 'windows' },
  linux: { title: 'Linux', badgeLabel: 'Linux', color: 'f84e29', logo: 'linux' },
  android: { title: 'Android', badgeLabel: 'APK', color: '168039', logo: 'android' },
};

function shieldsBadge(label, message, color, logo) {
  const esc = (text) => encodeURIComponent(text).replace(/-/g, '--').replace(/_/g, '__');
  return `https://img.shields.io/badge/${esc(label)}-${esc(message)}-${color}.svg?logo=${logo}&logoColor=white`;
}

/** Renders bilingual changelog followed by FlClash-style compact icon download table and SHA-256 table. */
export function renderReleaseNotes({
  version,
  tag,
  previousTag = null,
  repo = DEFAULT_REPO,
  changelogSection,
  installers,
}) {
  parseVersion(version);
  const lines = [
    changelogSection.trim(),
    '',
    '---',
    '',
    '<div align="center">',
    '',
    `[![Release Downloads](https://img.shields.io/github/downloads/${repo}/${tag}/total?style=flat-square&logo=github)](https://github.com/${repo}/releases/tag/${tag})`,
    '',
    '</div>',
    '',
    '**按操作系统点击图标下载 / Download based on your OS:**',
    '',
  ];

  if (installers.length === 0) {
    lines.push(
      `> 当前未扫描到本地安装包，请先运行 \`pnpm tauri build\` 或前往 [GitHub Releases](https://github.com/${repo}/releases/tag/${tag}) 查看。`,
      '',
    );
  } else {
    lines.push(
      '<div align="left">',
      '<table>',
      '  <thead align="left">',
      '    <tr>',
      '      <th>OS</th>',
      '      <th>Download</th>',
      '    </tr>',
      '  </thead>',
      '  <tbody align="left">',
    );
    for (const osKey of ['macos', 'windows', 'linux', 'android']) {
      const group = installers.filter((item) => item.os === osKey);
      if (group.length === 0) continue;
      const meta = OS_META[osKey];
      const badgeLines = group.map((item) => {
        const url = `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(item.name)}`;
        const img = shieldsBadge(
          item.badgeLabel ?? meta.badgeLabel,
          item.badgeMessage,
          item.badgeColor ?? meta.color,
          item.badgeLogo ?? meta.logo,
        );
        return `        <a href="${url}"><img src="${img}" alt="${meta.title} ${item.label}"></a>`;
      });
      lines.push(
        '    <tr>',
        `      <td>${meta.title}</td>`,
        '      <td>',
        badgeLines.join('<br>\n'),
        '      </td>',
        '    </tr>',
      );
    }
    lines.push('  </tbody>', '</table>', '</div>', '');
  }

  if (installers.length > 0) {
    lines.push(
      '<details>',
      '<summary>🔐 SHA-256 校验和 / Checksums</summary>',
      '',
      '| File | Size | SHA-256 |',
      '|---|---:|---|',
    );
    for (const item of installers) {
      lines.push(`| \`${item.name}\` | ${formatBytes(item.size)} | \`${item.sha256}\` |`);
    }
    lines.push('', '</details>', '');
  }

  if (previousTag) {
    lines.push(
      `**Full Changelog**: https://github.com/${repo}/compare/${previousTag}...${tag}`,
      '',
    );
  } else {
    lines.push(
      `**Full Release**: https://github.com/${repo}/releases/tag/${tag}`,
      '',
    );
  }

  return `${lines.join('\n').trimEnd()}\n`;
}

/** Builds the `gh release create` argument array for manual review or `--publish`. */
export function buildGhReleaseCommand({
  tag,
  title,
  notesFile,
  files,
  repo = DEFAULT_REPO,
  draft = false,
  prerelease = false,
}) {
  const args = [
    'release',
    'create',
    tag,
    ...files,
    '--repo',
    repo,
    '--title',
    title,
    '--notes-file',
    notesFile,
  ];
  if (draft) args.push('--draft');
  if (prerelease) args.push('--prerelease');
  return args;
}

function shellQuote(arg) {
  return /^[A-Za-z0-9_./:@=-]+$/.test(arg) ? arg : `'${arg.replace(/'/g, `'\\''`)}'`;
}

async function loadJsonSource(source) {
  if (!source) return null;
  if (/^https?:\/\//i.test(source)) {
    const res = await fetch(source);
    if (!res.ok) {
      throw new Error(`无法下载已有清单 ${source}: HTTP ${res.status}`);
    }
    return await res.json();
  }
  return JSON.parse(readFileSync(source, 'utf8'));
}

function latestGitTag(rootDir, excludeTag) {
  const result = spawnSync('git', ['tag', '--sort=-v:refname'], {
    cwd: rootDir,
    encoding: 'utf8',
  });
  if (result.status !== 0) return null;
  const tags = result.stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((tag) => tag && tag !== excludeTag);
  return tags[0] ?? null;
}

function parsePrepareFlags(args) {
  const options = {
    tag: null,
    repo: DEFAULT_REPO,
    from: [],
    out: null,
    mergeLatest: null,
    mergeAndroid: null,
    previousTag: null,
    dryRun: false,
    publish: false,
    draft: false,
    prerelease: false,
  };
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    const next = () => {
      if (i + 1 >= args.length) throw new Error(`参数 ${arg} 缺少取值`);
      i += 1;
      return args[i];
    };
    switch (arg) {
      case '--tag':
        options.tag = next();
        break;
      case '--repo':
        options.repo = next();
        break;
      case '--from':
        options.from.push(next());
        break;
      case '--out':
        options.out = next();
        break;
      case '--merge-latest':
        options.mergeLatest = next();
        break;
      case '--merge-android':
        options.mergeAndroid = next();
        break;
      case '--prev-tag':
        options.previousTag = next();
        break;
      case '--dry-run':
        options.dryRun = true;
        break;
      case '--publish':
        options.publish = true;
        break;
      case '--draft':
        options.draft = true;
        break;
      case '--prerelease':
        options.prerelease = true;
        break;
      default:
        throw new Error(`未知参数: ${arg}`);
    }
  }
  return options;
}

async function runPrepare(rootDir, rawArgs) {
  const opts = parsePrepareFlags(rawArgs);
  const workspaceVersion = readWorkspaceVersion(rootDir);
  const version = opts.tag ? parseVersion(opts.tag) : workspaceVersion;
  const tag = opts.tag?.startsWith('v') ? opts.tag : `v${version}`;
  const prerelease = opts.prerelease || version.includes('-');
  const pubDate = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');

  const scanDirs =
    opts.from.length > 0
      ? opts.from.map((dir) => {
          const resolved = resolve(rootDir, dir);
          if (!existsSync(resolved)) {
            throw new Error(`指定的产物目录不存在: ${resolved}`);
          }
          return resolved;
        })
      : DEFAULT_SCAN_DIRS.map((dir) => join(rootDir, dir)).filter((dir) => existsSync(dir));

  const collected = collectArtifacts(scanDirs, { version });
  const changelogPath = join(rootDir, 'CHANGELOG.md');
  const changelogText = existsSync(changelogPath) ? readFileSync(changelogPath, 'utf8') : '';
  const changelogSection = extractChangelogSection(changelogText, version);

  const existingLatest = await loadJsonSource(opts.mergeLatest);
  const existingAndroid = await loadJsonSource(opts.mergeAndroid);

  const latestManifest =
    collected.updaterPackages.length > 0 || existingLatest
      ? buildLatestManifest({
          version,
          tag,
          repo: opts.repo,
          notes: changelogSection,
          pubDate,
          updaterPackages: collected.updaterPackages,
          existing: existingLatest,
        })
      : null;

  const androidManifest =
    collected.androidApks.length > 0 || existingAndroid
      ? buildAndroidManifest({
          version,
          tag,
          repo: opts.repo,
          notes: changelogSection,
          pubDate,
          androidApks: collected.androidApks,
          existing: existingAndroid,
        })
      : null;

  const previousTag = opts.previousTag ?? latestGitTag(rootDir, tag);
  const releaseNotes = renderReleaseNotes({
    version,
    tag,
    previousTag,
    repo: opts.repo,
    changelogSection,
    installers: collected.installers,
  });

  if (opts.dryRun) {
    console.log('=== RELEASE_NOTES.md (dry-run) ===\n');
    console.log(releaseNotes);
    if (latestManifest) {
      console.log('=== latest.json (dry-run) ===\n');
      console.log(JSON.stringify(latestManifest, null, 2));
    }
    if (androidManifest) {
      console.log('\n=== latest-android.json (dry-run) ===\n');
      console.log(JSON.stringify(androidManifest, null, 2));
    }
    return;
  }

  const outDir = resolve(rootDir, opts.out ?? join('dist/release', tag));
  mkdirSync(outDir, { recursive: true });

  const notesFile = join(outDir, 'RELEASE_NOTES.md');
  writeFileSync(notesFile, releaseNotes, 'utf8');

  const releaseAssets = [];
  for (const entry of collected.uploadEntries) {
    const destPath = join(outDir, entry.name);
    if (resolve(entry.srcPath) !== resolve(destPath)) {
      copyFileSync(entry.srcPath, destPath);
    }
    releaseAssets.push(destPath);
  }

  if (collected.installers.length > 0) {
    const sumsPath = join(outDir, 'SHA256SUMS.txt');
    const sums = `${collected.installers.map((item) => `${item.sha256}  ${item.name}`).join('\n')}\n`;
    writeFileSync(sumsPath, sums, 'utf8');
    releaseAssets.push(sumsPath);
  }

  if (latestManifest) {
    const latestPath = join(outDir, 'latest.json');
    writeFileSync(latestPath, `${JSON.stringify(latestManifest, null, 2)}\n`, 'utf8');
    releaseAssets.push(latestPath);
  }

  if (androidManifest) {
    const androidPath = join(outDir, 'latest-android.json');
    writeFileSync(androidPath, `${JSON.stringify(androidManifest, null, 2)}\n`, 'utf8');
    releaseAssets.push(androidPath);
  }

  const ghArgs = buildGhReleaseCommand({
    tag,
    title: `RoveTerm ${tag}`,
    notesFile: relative(rootDir, notesFile),
    files: releaseAssets.map((file) => relative(rootDir, file)),
    repo: opts.repo,
    draft: opts.draft,
    prerelease,
  });

  console.log(`Prepared release artifacts in ${relative(rootDir, outDir)}`);
  console.log(`- Installers: ${collected.installers.length}`);
  console.log(
    `- Desktop updater platforms: ${latestManifest ? Object.keys(latestManifest.platforms).join(', ') || 'none' : 'none'}`,
  );
  console.log(
    `- Android ABIs: ${androidManifest ? Object.keys(androidManifest.platforms).join(', ') || 'none' : 'none'}`,
  );
  console.log(`\nReview & edit ${relative(rootDir, notesFile)}, then publish with:\n`);
  console.log(`  gh ${ghArgs.map(shellQuote).join(' ')}\n`);

  if (opts.publish) {
    const result = spawnSync('gh', ghArgs, { cwd: rootDir, stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(`gh release create 退出码 ${result.status}`);
    }
  }
}

function usage() {
  return [
    'Usage:',
    '  node scripts/release.mjs bump <version>',
    '  node scripts/release.mjs prepare [--tag vX.Y.Z] [--from <dir>...] [--merge-latest <path|url>] [--merge-android <path|url>] [--out <dir>] [--prev-tag <tag>] [--dry-run] [--publish] [--draft] [--prerelease]',
  ].join('\n');
}

async function main() {
  const rootDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const [command, ...rest] = process.argv.slice(2);
  if (!command || command === '-h' || command === '--help') {
    console.log(usage());
    return;
  }
  switch (command) {
    case 'bump': {
      if (rest.length !== 1) throw new Error('用法: pnpm release:bump <version>');
      const { version } = bumpWorkspaceVersion(rootDir, rest[0]);
      console.log(`Updated Cargo.toml and package.json to ${version}`);
      break;
    }
    case 'prepare': {
      await runPrepare(rootDir, rest);
      break;
    }
    default:
      throw new Error(`未知子命令: ${command}\n\n${usage()}`);
  }
}

const isEntry =
  process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isEntry) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
