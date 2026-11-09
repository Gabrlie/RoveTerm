import { mount } from 'svelte';
import '@xterm/xterm/css/xterm.css';
import './styles/tokens.css';
import './styles/fonts.css';
import App from './App.svelte';
import { prefs } from './lib/settings/prefs.svelte';

const target = document.getElementById('app');
if (!target) throw new Error('missing #app mount point');

// Terminals are created with the stored preferences, so load them before the first view mounts.
await prefs.load().catch((error: unknown) => console.warn('preferences unavailable, using defaults', error));

export default mount(App, { target });
