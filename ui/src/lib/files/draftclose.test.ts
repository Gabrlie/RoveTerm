import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { protectDrafts, type CloseChoice } from './draftclose.ts';
const draft = () => ({ id: 'a', text: 'draft', original: 'server', busy: false });

test('cancel retains drafts, discard never writes, clean drafts do not ask', async () => {
  let writes = 0;
  const save = async () => { writes++; return true; };
  assert.equal(await protectDrafts([draft()], async () => 'cancel', save, () => {}), false);
  assert.equal(await protectDrafts([draft()], async () => 'discard', save, () => {}), true);
  assert.equal(await protectDrafts([{ ...draft(), text: 'server' }], async () => { throw Error('must not ask'); }, save, () => {}), true);
  assert.equal(writes, 0);
});
test('a conflict during save prevents close and selects the failing draft', async () => {
  let failed = '';
  assert.equal(await protectDrafts([draft()], async () => 'save', async () => false, (item) => { failed = item.id; }), false);
  assert.equal(failed, 'a');
});
test('batch save protects a draft edited while another one saves', async () => {
  const first = draft();
  const second = { ...draft(), id: 'b' };
  assert.equal(await protectDrafts([first, second], async () => 'save' as CloseChoice, async (item) => {
    item.original = item.text;
    if (item.id === 'b') first.text = 'newer draft';
    return true;
  }, () => {}), false);
});
test('in-flight writes prevent close without prompting', async () => {
  assert.equal(await protectDrafts([{ ...draft(), busy: true }], async () => { throw Error('must not ask'); }, async () => true, () => {}), false);
});
