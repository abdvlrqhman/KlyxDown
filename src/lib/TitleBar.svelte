<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import { api } from './api';
  import { app } from './store.svelte';

  const win = getCurrentWindow();
  let maximized = $state(false);
  let link = $state('');

  onMount(() => {
    win.isMaximized().then((m) => (maximized = m));
    const off = win.onResized(async () => (maximized = await win.isMaximized()));
    return () => {
      off.then((f) => f());
    };
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    app.openAdd(link.trim());
    link = '';
  }

  // Minimizing sends the app to the tray when that setting is on (the default).
  const minimize = () => (app.settings?.minimizeToTray ?? true ? win.hide() : win.minimize());
  const anyRunning = $derived(app.running.length > 0);
  const anyPaused = $derived(app.paused.length > 0);
</script>

<header class="titlebar panel" data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <Logo size={22} active={anyRunning} />
    <span data-tauri-drag-region>KlyxDown</span>
  </div>

  <form class="linkbox slot" onsubmit={submit}>
    <Icon name="link" />
    <input bind:value={link} type="text" placeholder="Paste a link or magnet" aria-label="Paste a link or magnet" autocomplete="off" spellcheck="false" />
  </form>
  <button class="key go" type="button" onclick={() => (app.openAdd(link.trim()), (link = ''))}>
    <Icon name="plus" />Add
  </button>

  <div class="drag" data-tauri-drag-region></div>

  {#if anyRunning || !anyPaused}
    <button class="key" type="button" disabled={!anyRunning} onclick={() => app.run(api.pauseAll)}><Icon name="pause" />Pause all</button>
  {:else}
    <button class="key" type="button" onclick={() => app.run(api.resumeAll)}><Icon name="play" />Resume all</button>
  {/if}
  <button
    class="key sq"
    type="button"
    aria-label="Settings"
    title="Settings (Ctrl ,)"
    aria-pressed={app.settingsOpen}
    onclick={() => (app.settingsOpen = !app.settingsOpen)}
  >
    <Icon name="gear-six" />
  </button>

  <div class="wc" role="group" aria-label="Window">
    <button type="button" aria-label="Minimize" title={app.settings?.minimizeToTray ? 'Minimize to tray' : 'Minimize'} onclick={minimize}>
      <Icon name="minus" size={14} />
    </button>
    <button type="button" aria-label={maximized ? 'Restore' : 'Maximize'} title={maximized ? 'Restore' : 'Maximize'} onclick={() => win.toggleMaximize()}>
      <Icon name={maximized ? 'corners-in' : 'corners-out'} size={14} />
    </button>
    <button class="close" type="button" aria-label="Close" title="Close" onclick={() => win.close()}>
      <Icon name="x" size={14} />
    </button>
  </div>
</header>

<style>
  .titlebar {
    grid-area: top;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 52px;
    padding: 0 10px 0 14px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-right: 6px;
    font-weight: 800;
    font-size: 15px;
    font-stretch: 118%;
    letter-spacing: -0.01em;
  }
  .linkbox {
    flex: 1;
    max-width: 440px;
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--ink-2);
  }
  .linkbox input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--ink);
  }
  .linkbox input::placeholder {
    color: var(--ink-3);
  }
  .drag {
    flex: 1;
    align-self: stretch;
  }
  .wc {
    display: flex;
    gap: 4px;
    margin-left: 6px;
    padding: 3px;
    border-radius: 8px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
  }
  .wc button {
    width: 30px;
    height: 26px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--ink-2);
    transition: background-color 0.15s, color 0.15s;
  }
  .wc button:hover {
    background: var(--key);
    color: var(--ink);
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo);
  }
  .wc .close:hover {
    background: var(--warn);
    color: var(--on-accent);
  }
</style>
