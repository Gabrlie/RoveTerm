import { mount } from 'svelte';
import '@xterm/xterm/css/xterm.css';
import './styles/tokens.css';
import './styles/controls.css';
import './styles/fonts.css';
import App from './App.svelte';
import { prefs } from './lib/settings/prefs.svelte';
import { isMacDesktop } from './lib/platform';
import { initInsets } from './lib/layout/insets';
import { forwardConsole } from './lib/ui/consolelog';
import { logWrite, mobileInsets, onMobileInsets } from './lib/ipc';

// Warnings and errors of the interface also go to the log files.
forwardConsole(logWrite);

// macOS draws its window buttons over the top-left corner (overlay title bar).
if (isMacDesktop) document.documentElement.dataset.platform = 'mac';
initInsets({ fetch: mobileInsets, listen: onMobileInsets });

const target = document.getElementById('app');
if (!target) throw new Error('missing #app mount point');

// Terminals are created with the stored preferences, so load them before the first view mounts.
// When they cannot be read yet, the defaults show until a later attempt succeeds; edits always
// start from the stored preferences, so the defaults are never saved over them.
async function loadPrefs(attempt: number): Promise<void> {
  try {
    await prefs.load();
  } catch (error) {
    console.warn(`preferences unavailable (attempt ${attempt + 1})`, error);
    if (attempt < 5) setTimeout(() => void loadPrefs(attempt + 1), 1000 * 2 ** attempt);
  }
}
await loadPrefs(0);

export default mount(App, { target });
