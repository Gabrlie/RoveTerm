import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canChangeTerminal, canPrestart, choiceKey, currentChoice, effortPill, fastIndicator, modelGroups, optionKey, slashItems, slashQuery, speedToggle, splitOptions } from './composer.ts';
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

test('a speed setting with a fast tier becomes the fast switch', () => {
  const tier = option('service_tier', null, 'select', ['default', 'flex', 'fast']);
  const split = splitOptions([tier]);
  assert.equal(split.speed, tier);
  assert.equal(speedToggle(tier, 'default'), 'fast');
  assert.equal(speedToggle(tier, 'flex'), 'fast');
  assert.equal(speedToggle(tier, 'fast'), 'default');
  assert.equal(splitOptions([option('service_tier', null, 'select', ['flex', 'default'])]).speed, null);
});

test('known settings and choices are named from the dictionary', () => {
  const effort = option('reasoning_effort', null, 'select', ['low', 'xhigh']);
  assert.equal(optionKey(effort), 'ai.effort');
  assert.equal(choiceKey(effort, 'xhigh'), 'ai.effort.xhigh');
  assert.equal(optionKey(option('model', 'model', 'select')), 'ai.model');
  assert.equal(choiceKey(option('model', 'model', 'select', ['max']), 'max'), null);
  assert.equal(choiceKey(option('sandbox', null, 'select'), 'workspace-write'), 'ai.sandbox.workspaceWrite');
  assert.equal(optionKey(option('custom', null, 'select')), null);
});

test('the model trigger uses display names and falls back for unlisted settings', () => {
  const model = option('model', 'model', 'select');
  model.choices = [{ value: 'gpt', name: 'GPT', description: null, group: 'OpenAI' }];
  assert.equal(currentChoice(model, () => 'gpt')?.name, 'GPT');
  assert.equal(currentChoice(model, () => 'gpt')?.group, 'OpenAI');
  assert.equal(currentChoice(model, () => 'custom/model')?.name, 'custom/model');
  assert.equal(currentChoice(model, () => ''), null);
  assert.equal(currentChoice(null, () => 'gpt'), null);
  assert.equal(effortPill('High'), '· High');
  assert.equal(effortPill('高'), '· 高');
  assert.equal(effortPill(null), '');
});

test('the fast indicator reflects speed tiers and fast boolean settings', () => {
  const tier = option('service_tier', null, 'select', ['standard', 'fast']);
  const fast = option('fast_mode', null, 'boolean');
  const other = option('verbose', null, 'boolean');
  assert.equal(fastIndicator(tier, [], () => 'fast'), 'on');
  assert.equal(fastIndicator(tier, [], () => 'standard'), 'off');
  assert.equal(fastIndicator(null, [fast], () => 'true'), 'on');
  assert.equal(fastIndicator(null, [fast], () => 'false'), 'off');
  assert.equal(fastIndicator(null, [other], () => 'true'), null);
});

test('model search matches display names and provider groups and keeps group order', () => {
  const gpt = { value: 'gpt', name: 'GPT Mini', description: null, group: 'OpenAI' };
  const opus = { value: 'opus', name: 'Opus', description: null, group: 'Anthropic' };
  const other = { value: 'other', name: 'GPT Large', description: null, group: 'OpenAI' };
  const choices = [gpt, opus, other];
  assert.deepEqual(modelGroups(choices, ''), [['OpenAI', [gpt, other]], ['Anthropic', [opus]]]);
  assert.deepEqual(modelGroups(choices, '  MINI  '), [['OpenAI', [gpt]]]);
  assert.deepEqual(modelGroups(choices, 'aNTHropic'), [['Anthropic', [opus]]]);
  assert.deepEqual(modelGroups(choices, 'missing'), []);
});

test('eligible ACP chats prestart once per agent, including chats about remote hosts', () => {
  const chat = { external: null, context: { kind: 'host' }, id: null, starting: false, prestartedAgent: null };
  assert.equal(canPrestart('acp:codex', true, chat), true);
  assert.equal(canPrestart('acp:codex', false, chat), false);
  assert.equal(canPrestart('builtin:openai', true, chat), false);
  assert.equal(canPrestart(null, true, chat), false);
  assert.equal(canPrestart('acp:codex', true, { ...chat, external: 'Codex' }), false);
  assert.equal(canPrestart('acp:codex', true, { ...chat, context: { kind: 'fleet' } }), false);
  assert.equal(canPrestart('acp:codex', true, { ...chat, starting: true }), false);
  assert.equal(canPrestart('acp:codex', true, { ...chat, id: 'session' }), false);
  const failed = { ...chat, prestartedAgent: 'acp:codex' };
  assert.equal(canPrestart('acp:codex', true, failed), false);
  assert.equal(canPrestart('acp:claude', true, failed), true);
});

test('terminal access stays editable for empty chats through prestart and locks after the first message', () => {
  const idle = { id: 'prepared', starting: false, hasUserMessage: false, busy: false };
  assert.equal(canChangeTerminal(idle), true);
  assert.equal(canChangeTerminal({ ...idle, id: null, starting: true }), true);
  assert.equal(canChangeTerminal({ ...idle, id: null, hasUserMessage: true }), false);
  assert.equal(canChangeTerminal({ ...idle, busy: true }), false);
});
