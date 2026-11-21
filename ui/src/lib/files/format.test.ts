import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { DirEntry } from '../ipc';
import { isEditableTextFile } from './format.ts';

const entry = (name: string, size = 64, kind: DirEntry['kind'] = 'file'): DirEntry =>
  ({ name, size, kind, modified: null, permissions: null });

test('recognizes text, source code, and markup case-insensitively', () => {
  for (const name of ['notes.TXT', 'app.LOG', 'main.rs', 'types.d.ts', 'site.SVG', 'server.JSONL', 'id_ed25519.pub']) {
    assert.equal(isEditableTextFile(entry(name)), true, name);
  }
});

test('recognizes configuration files, systemd units, and dotfiles', () => {
  for (const name of ['NGINX.CONF', 'backup.service', 'backup.timer', '.env', '.ENV.Production', '.gitignore', '.bash_profile', '.tmux.conf']) {
    assert.equal(isEditableTextFile(entry(name)), true, name);
  }
});

test('recognizes known extensionless files', () => {
  for (const name of ['Dockerfile', 'GNUmakefile', 'README', 'LICENSE', 'hosts', 'authorized_keys', 'config']) {
    assert.equal(isEditableTextFile(entry(name)), true, name);
  }
});

test('excludes binary suffixes, including on environment files, and unknown names', () => {
  for (const name of ['bundle.tar.gz', 'photo.AVIF', 'song.flac', 'movie.mov', 'doc.docx', 'script.pyc', 'data.sqlite3', 'font.woff2', '.env.zip', 'unknown', 'file.unknown']) {
    assert.equal(isEditableTextFile(entry(name)), false, name);
  }
});

test('accepts only regular files within the backend text size limit', () => {
  assert.equal(isEditableTextFile(entry('empty.txt', 0)), true);
  assert.equal(isEditableTextFile(entry('exact.txt', 2 * 1024 * 1024)), true);
  assert.equal(isEditableTextFile(entry('large.txt', 2 * 1024 * 1024 + 1)), false);
  for (const kind of ['dir', 'symlink', 'other'] as const) {
    assert.equal(isEditableTextFile(entry('notes.txt', 64, kind)), false, kind);
  }
});
