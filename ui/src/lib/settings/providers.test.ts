import { test } from 'node:test';
import assert from 'node:assert/strict';
import { addModel, customAgentId, fromPreset, mergeListed, PRESETS, providerId, providerModels } from './providers.ts';

test('ids are slugs that do not collide', () => {
  assert.equal(providerId('OpenAI', []), 'openai');
  assert.equal(providerId('OpenAI', ['openai', 'openai-2']), 'openai-3');
  assert.equal(providerId('我的 服务', []), 'provider');
  assert.equal(providerId('  Local / LLM!  ', []), 'local-llm');
  assert.ok(providerId('x'.repeat(80), []).length <= 40);
  assert.match(customAgentId('My Agent', []), /^custom-my-agent$/);
  assert.equal(customAgentId('', ['custom']), 'custom-2');
});

test('presets fill the form but leave the model to the user', () => {
  const ollama = PRESETS.find((preset) => preset.id === 'ollama');
  assert.ok(ollama?.keyless);
  const provider = fromPreset(ollama, ['ollama']);
  assert.equal(provider.id, 'ollama-2');
  assert.equal(provider.base_url, 'http://localhost:11434/v1');
  assert.equal(provider.model, '');
  for (const preset of PRESETS) assert.ok(preset.base_url === '' || /^https?:\/\//.test(preset.base_url));
});

test('checked models become the default and the others', () => {
  assert.deepEqual(providerModels(['a', 'b', 'c'], 'b'), { model: 'b', models: ['a', 'c'] });
  assert.deepEqual(providerModels(['a', ' b ', 'a'], 'x'), { model: 'a', models: ['b'] });
  assert.deepEqual(providerModels([], ''), { model: '', models: [] });
  assert.deepEqual(mergeListed(['z', 'a', 'm'], ['m', 'custom']), ['custom', 'm', 'a', 'z']);
});

test('adding a manual model selects and stars the first model without a fetch', () => {
  assert.deepEqual(addModel(' local/model ', [], [], ''), {
    picked: ['local/model'],
    listed: ['local/model'],
    star: 'local/model',
  });
});

test('manual model ids deduplicate and preserve the current default and fetched options', () => {
  const first = addModel('manual', ['default'], ['fetched', 'default'], 'default');
  assert.deepEqual(addModel(' manual ', first.picked, first.listed, first.star), {
    picked: ['default', 'manual'],
    listed: ['default', 'manual', 'fetched'],
    star: 'default',
  });
  assert.deepEqual(mergeListed(['remote', 'manual'], first.picked), ['default', 'manual', 'remote']);
});
