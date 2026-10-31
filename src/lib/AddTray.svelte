<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { open } from '@tauri-apps/plugin-dialog';
  import Icon from './Icon.svelte';
  import Select from './Select.svelte';
  import { api, errorText } from './api';
  import { bytes } from './format';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { Probe } from './types';

  // Downloads caught by the browser extension carry its cookies and referrer.
  const capture = app.capture;
  let url = $state(app.addPrefill);
  let found = $state<Probe | null>(null);
  let name = $state('');
  let dir = $state(app.settings?.downloadDir ?? '');
  let dirPicked = false;
  let connections = $state(app.settings?.connections ?? 8);
  let busy = $state(false);
  let error = $state('');
  let input: HTMLInputElement;
  const CONNECTIONS = [1, 2, 4, 8, 16, 32].map((n) => ({ value: n, label: n === 1 ? '1 connection' : `${n} connections` }));

  onMount(() => {
    if (url) check();
    else input.focus();
  });

  async function check() {
    if (!url.trim()) return void (error = 'Paste a link first.');
    error = '';
    busy = true;
    try {
      found = await api.probe(url.trim(), capture?.headers ?? []);
      name = capture?.name ?? found.name;
      if (!dirPicked) dir = await api.suggestDir(name);
    } catch (e) {
      found = null;
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function add(start: boolean) {
    if (!found) return;
    busy = true;
    try {
      await api.add({
        url: found.url,
        dir,
        fileName: name.trim() && name.trim() !== found.name ? name.trim() : null,
        connections: found.resumable ? connections : 1,
        start,
        headers: capture?.headers ?? [],
      });
      if (capture?.id) await api.captureDone(capture.id, true);
      app.closeAdd(true);
      app.toast(start ? `Started ${name}` : `Added ${name}, paused`);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true, defaultPath: dir || undefined, title: 'Save downloads to' });
    if (typeof picked === 'string') {
      dir = picked;
      dirPicked = true;
    }
  }

  const close = () => app.closeAdd();
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="add-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <h2 id="add-title">{capture ? `${capture.browser} sent a download` : 'Add a download'}</h2>
    <button class="key sm sq" type="button" aria-label="Close" onclick={close}><Icon name="x" /></button>
  </header>

  <form class="field" onsubmit={(e) => (e.preventDefault(), found ? add(true) : check())}>
    <label for="add-url">Link</label>
    <div class="url-row">
      <input id="add-url" class="slot" bind:this={input} bind:value={url} type="text" placeholder="https://" autocomplete="off" spellcheck="false" oninput={() => (found = null)} />
      <button class="key" type="submit" disabled={busy}>{found ? 'Check again' : 'Check link'}</button>
    </div>
    {#if busy}<span class="scan" aria-label="Checking"><i></i><i></i><i></i><i></i><i></i><i></i></span>{/if}
    {#if error}<span class="err" role="alert">{error}</span>{/if}
  </form>

  {#if found}
    <div class="found" in:fly={{ y: 6, duration: ms(220) }}>
      <div class="field">
        <label for="add-name">File name</label>
        <div class="name-row">
          <input id="add-name" class="slot" bind:value={name} type="text" spellcheck="false" />
          <span class="size num">{bytes(found.size)}</span>
        </div>
      </div>
      <div class="two">
        <div class="field">
          <label for="add-dir">Save to</label>
          <div class="dir-row">
            <input id="add-dir" class="slot" bind:value={dir} type="text" spellcheck="false" />
            <button class="key sm" type="button" onclick={pickFolder}><Icon name="folder-open" />Change</button>
          </div>
        </div>
        <div class="field">
          <label for="add-conns">Connections</label>
          <Select id="add-conns" label="Connections" bind:value={connections} disabled={!found.resumable} options={CONNECTIONS} />
        </div>
      </div>
      <p class="hint">
        {found.resumable
          ? 'The server supports resuming, so pausing is safe and several connections can share the work.'
          : 'This server cannot resume. It downloads on one connection, and pausing restarts it.'}
      </p>
    </div>
  {/if}

  <footer>
    <button class="key sm" type="button" onclick={close}>{capture?.id ? `Let ${capture.browser} download it` : 'Cancel'}</button>
    <button class="key sm" type="button" disabled={!found || busy} onclick={() => add(false)}>Add paused</button>
    <button class="key sm go" type="button" disabled={!found || busy} onclick={() => add(true)}>Start download</button>
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: color-mix(in srgb, var(--recess) 55%, transparent);
  }
  .tray {
    position: fixed;
    z-index: 31;
    top: 50%;
    left: 50%;
    /* `translate` composes with the fly transition's `transform`, so the tray stays centered while it animates. */
    translate: -50% -50%;
    width: min(580px, calc(100% - 32px));
    max-height: calc(100% - 32px);
    padding: 16px;
    display: grid;
    gap: 14px;
    box-shadow: inset 0 1px 0 var(--hi), 0 30px 60px -20px var(--shadow), 0 0 0 1px var(--lo);
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  header h2 {
    flex: 1;
    margin: 0;
    font-size: 15px;
    font-stretch: 90%;
  }
  footer {
    justify-content: flex-end;
  }
  .field {
    display: grid;
    gap: 6px;
  }
  label {
    font-size: 11.5px;
    font-weight: 600;
    font-stretch: 85%;
    color: var(--ink-2);
  }
  .url-row,
  .dir-row,
  .name-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .url-row .slot,
  .dir-row .slot,
  .name-row .slot {
    flex: 1;
    min-width: 0;
  }
  .size {
    color: var(--ink-2);
    white-space: nowrap;
  }
  .found {
    display: grid;
    gap: 12px;
  }
  .two {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 150px;
    gap: 12px;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--ink-2);
  }
  .err {
    font-size: 12px;
    color: var(--warn);
  }
  .scan {
    display: flex;
    gap: 3px;
  }
  .scan i {
    width: 6px;
    height: 6px;
    border-radius: 1px;
    background: var(--led-off);
    animation: scan 0.9s infinite;
  }
  .scan i:nth-child(2) { animation-delay: 0.1s; }
  .scan i:nth-child(3) { animation-delay: 0.2s; }
  .scan i:nth-child(4) { animation-delay: 0.3s; }
  .scan i:nth-child(5) { animation-delay: 0.4s; }
  .scan i:nth-child(6) { animation-delay: 0.5s; }
  @keyframes scan {
    0%,
    60%,
    100% {
      background: var(--led-off);
    }
    20% {
      background: var(--accent);
    }
  }
</style>
