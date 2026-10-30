import '@fontsource-variable/archivo/wdth.css';
import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';

const target = document.getElementById('app');
if (!target) throw new Error('Missing #app element');
const start = () => mount(App, { target });

// In a plain browser during development, fake the Rust side with sample data.
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
  import('./lib/dev-preview').then((m) => (m.installPreview(), start()));
} else {
  start();
}
