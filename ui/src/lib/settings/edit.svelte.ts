// Settings apply as soon as they change: each edit stores a modified copy of the preferences and
// reports a rejected value in `settingsError`.
import type { DevicePrefs, Preferences } from '../ipc';
import { errorText } from '../i18n';
import { applyWindowMaterial } from '../ui/window';
import { prefs } from './prefs.svelte';

export const settingsError = $state({ text: '' });

/** Stores the preferences after `change` edits a copy of them. */
export async function editPrefs(change: (draft: Preferences) => void): Promise<void> {
  const draft = structuredClone($state.snapshot(prefs.value)) as Preferences;
  change(draft);
  try {
    await prefs.save(draft);
    settingsError.text = '';
  } catch (error) {
    settingsError.text = errorText(error);
  }
}

/** Stores this device's preferences after `change` edits a copy, and applies the window material. */
export async function editDevice(change: (draft: DevicePrefs) => void): Promise<void> {
  const draft = structuredClone($state.snapshot(prefs.device)) as DevicePrefs;
  change(draft);
  try {
    const materialChanged = draft.window_material !== prefs.device.window_material;
    await prefs.saveDevice(draft);
    if (materialChanged) await applyWindowMaterial(prefs.device.window_material);
    settingsError.text = '';
  } catch (error) {
    settingsError.text = errorText(error);
  }
}
