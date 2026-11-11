import { test } from 'node:test';
import assert from 'node:assert/strict';
import { effortIndex, slashItems, slashQuery, splitOptions } from './composer.ts';
import type { ConfigOption } from '../ipc.ts';

const option = (id: string, category: string | null, kind: 'select' | 'boolean', values: string[] = []): ConfigOption => ({
  id,
  name: id,
  category,
  kind,
  current: values[0] ?? 'false',
  choices: values.map((value) => ({ value, name: value, description: null })),
});

test('settings split between the model popover and the slash menu', () => {
  const model = option('model', 'model', 'select', ['a', 'b']);
  const effort = option('reasoning_effort', null, 'select', ['low', 'high']);
  const fast = option('fast', null, 'boolean');
  const agent = option('agent', null, 'select', ['plan', 'build']);
  const split = splitOptions([model, effort, fast, agent]);
  assert.equal(split.model, model);
  assert.equal(split.effort, effort);
  assert.deepEqual(split.toggles, [fast]);
  assert.deepEqual(split.others, [agent]);
  assert.equal(effortIndex(effort, 'high'), 1);
  assert.equal(effortIndex(effort, 'unknown'), 0);
});

test('the slash menu follows what is typed after the slash', () => {
  assert.equal(slashQuery('/'), '');
  assert.equal(slashQuery('/rev'), 'rev');
  assert.equal(slashQuery('/review this'), null);
  assert.equal(slashQuery('hello'), null);
  const commands = [
    { name: 'prereview', description: '', hint: null },
    { name: 'review', description: '', hint: null },
    { name: 'compact', description: '', hint: null },
  ];
  const agent = option('agent', null, 'select', ['plan']);
  const items = slashItems('rev', commands, [agent]);
  assert.deepEqual(
    items.map((item) => (item.kind === 'command' ? item.command.name : item.option.id)),
    ['review', 'prereview'],
  );
  assert.equal(slashItems('', commands, [agent]).length, 4);
  assert.deepEqual(slashItems('age', commands, [agent]), [{ kind: 'setting', option: agent }]);
});
