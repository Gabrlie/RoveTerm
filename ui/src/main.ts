import { mount } from 'svelte';
import '@xterm/xterm/css/xterm.css';
import './styles/tokens.css';
import App from './App.svelte';

const target = document.getElementById('app');
if (!target) throw new Error('missing #app mount point');

export default mount(App, { target });
