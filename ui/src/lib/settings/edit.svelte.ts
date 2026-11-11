// Settings apply as soon as they change: each edit reads the stored preferences, changes them and
// stores them, and reports a rejected value (or preferences that cannot be read) in
// `settingsError`.
import type { DevicePrefs, Preferences } from '../ipc';
import { errorText } from '../i18n';
import { applyWindowMaterial } from '../ui/window';
import { prefs } from './prefs.svelte';

export const settingsError = $state({ text: '' });

/** Stores the preferences after `change` edits a copy of them. */
export async function editPrefs(change: (draft: Preferences) => void): Promise<void> {
  try {
    const draft = await prefs.current();
    change(draft);
    await prefs.save(draft);
    settingsError.text = '';
  } catch (error) {
    settingsError.text = errorText(error);
  }
}

/** Stores this device's preferences after `change` edits a copy, and applies the window material. */
export async function editDevice(change: (draft: DevicePrefs) => void): Promise<void> {
  try {
    const before = prefs.device.window_material;
    const draft = await prefs.currentDevice();
    change(draft);
    const materialChanged = draft.window_material !== before;
    await prefs.saveDevice(draft);
    if (materialChanged) await applyWindowMaterial(prefs.device.window_material);
    settingsError.text = '';
  } catch (error) {
    settingsError.text = errorText(error);
  }
}
