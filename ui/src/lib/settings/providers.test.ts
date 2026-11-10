import { test } from 'node:test';
import assert from 'node:assert/strict';
import { customAgentId, fromPreset, PRESETS, providerId } from './providers.ts';

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
