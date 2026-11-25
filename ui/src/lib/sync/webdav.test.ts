import test from 'node:test';
import assert from 'node:assert/strict';
import {
  MASKED_PASSWORD_PLACEHOLDER,
  shouldClearHiddenWebdavPassword,
  splitWebdavUrl,
  webdavPasswordPlaceholder,
  webdavUrl,
} from './webdav.ts';

test('joins server URL and folder path', () => {
  assert.equal(webdavUrl('https://dav.jianguoyun.com/dav/', 'RoveTerm'), 'https://dav.jianguoyun.com/dav/RoveTerm');
  assert.equal(webdavUrl('https://dav.example.com/dav///', '/team/my sync/'), 'https://dav.example.com/dav/team/my%20sync');
});

test('splits stored WebDAV directory URLs for editing', () => {
  assert.deepEqual(splitWebdavUrl('https://dav.jianguoyun.com/dav/RoveTerm'), {
    server: 'https://dav.jianguoyun.com/dav/',
    folder: 'RoveTerm',
  });
  assert.deepEqual(splitWebdavUrl('https://cloud.example.com/remote.php/dav/files/alice/My%20Rove/'), {
    server: 'https://cloud.example.com/remote.php/dav/files/alice/',
    folder: 'My Rove',
  });
  assert.deepEqual(splitWebdavUrl('https://dav.example.com/'), {
    server: 'https://dav.example.com/',
    folder: '',
  });
});

test('round-trips typical WebDAV addresses', () => {
  const original = 'https://dav.jianguoyun.com/dav/RoveTerm';
  const { server, folder } = splitWebdavUrl(original);
  assert.equal(webdavUrl(server, folder), original);
});

test('masks the saved password while editing connected WebDAV storage', () => {
  assert.equal(MASKED_PASSWORD_PLACEHOLDER, '••••••••');
  assert.equal(webdavPasswordPlaceholder({ editing: true, isWebdav: true, connected: true }), MASKED_PASSWORD_PLACEHOLDER);
});

test('leaves the placeholder empty for setup, folder storage, and disconnected WebDAV storage', () => {
  assert.equal(webdavPasswordPlaceholder({ editing: false, isWebdav: true, connected: true }), '');
  assert.equal(webdavPasswordPlaceholder({ editing: true, isWebdav: false, connected: true }), '');
  assert.equal(webdavPasswordPlaceholder({ editing: true, isWebdav: true, connected: false }), '');
});

test('hiding clears only the password loaded from saved storage', () => {
  assert.equal(shouldClearHiddenWebdavPassword(true), true);
  assert.equal(shouldClearHiddenWebdavPassword(false), false);
});
