import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { editorArgs, editorProgram } from './externaleditor.ts';

test('argv keeps spaces, literal file placeholders, and does not parse a shell', () => {
  assert.deepEqual(editorArgs('--wait\r\n{file}\n\nname with spaces\n$(literal)'), ['--wait', '{file}', 'name with spaces', '$(literal)']);
});
test('application bundles clear arguments while executables preserve them', () => {
  assert.deepEqual(editorProgram(' /Applications/Code.app ', ['--wait', '{file}']), { program: '/Applications/Code.app', args: [] });
  assert.deepEqual(editorProgram('C:\\Editor.exe', ['{file}']), { program: 'C:\\Editor.exe', args: ['{file}'] });
});
