import { test } from 'node:test';
import assert from 'node:assert/strict';
import { webdavUrl } from './webdav.ts';

test('joins the server address and the folder', () => {
  assert.equal(webdavUrl('https://dav.jianguoyun.com/dav/', 'RoveTerm'), 'https://dav.jianguoyun.com/dav/RoveTerm');
  assert.equal(webdavUrl(' https://cloud.example/remote.php/dav/files/me ', ' /Apps/RoveTerm/ '), 'https://cloud.example/remote.php/dav/files/me/Apps/RoveTerm');
});

test('encodes folder names and allows none', () => {
  assert.equal(webdavUrl('https://dav.example//', '同步 数据'), 'https://dav.example/%E5%90%8C%E6%AD%A5%20%E6%95%B0%E6%8D%AE');
  assert.equal(webdavUrl('https://dav.example/', ''), 'https://dav.example');
});
