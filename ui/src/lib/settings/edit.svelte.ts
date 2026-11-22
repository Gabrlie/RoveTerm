// Settings apply as soon as they change: each edit reads the stored preferences, changes them and
// stores them, and reports a rejected value (or preferences that cannot be read) in
// `settingsError`.
import type { DevicePrefs, Preferences } from '../ipc';
import { errorText } from '../i18n';
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

/** Stores this device's preferences after `change` edits a copy of them. */
export async function editDevice(change: (draft: DevicePrefs) => void): Promise<void> {
  try {
    const draft = await prefs.currentDevice();
    change(draft);
    await prefs.saveDevice(draft);
    settingsError.text = '';
  } catch (error) {
    settingsError.text = errorText(error);
  }
}
