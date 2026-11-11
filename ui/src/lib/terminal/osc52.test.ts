import { test } from 'node:test';
import assert from 'node:assert/strict';
import { MAX_OSC52, osc52Text } from './osc52.ts';

const encode = (text: string): string => Buffer.from(text, 'utf8').toString('base64');

test('writes are decoded as UTF-8', () => {
  assert.equal(osc52Text(`c;${encode('hello')}`), 'hello');
  assert.equal(osc52Text(`;${encode('默认')}`), '默认');
  assert.equal(osc52Text(`p;${encode('x')}`), 'x');
});

test('queries and malformed sequences are refused', () => {
  assert.equal(osc52Text('c;?'), null);
  assert.equal(osc52Text('c;'), null);
  assert.equal(osc52Text('nothing'), null);
  assert.equal(osc52Text('c;***'), null);
  assert.equal(osc52Text(`zz;${encode('x')}`), null);
  assert.equal(osc52Text(`c;${encode('y'.repeat(MAX_OSC52 + 10))}`), null);
});
