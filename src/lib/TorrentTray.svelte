<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { open } from '@tauri-apps/plugin-dialog';
  import Icon from './Icon.svelte';
  import ScrollArea from './ScrollArea.svelte';
  import { api, errorText } from './api';
  import { bytes, kindIcon } from './format';
  import { t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { TorrentInfo } from './types';

  // A magnet link or .torrent: read its file list, pick files, download.
  let { torrent }: { torrent: { source: string } } = $props();

  let info = $state<TorrentInfo | null>(null);
  let picked = $state(new Set<number>());
  let dir = $state(app.settings?.downloadDir ?? '');
  let error = $state('');
  let busy = $state('');

  const pickedSize = $derived(info ? info.files.filter((f) => picked.has(f.index)).reduce((n, f) => n + f.size, 0) : 0);

  onMount(async () => {
    busy = torrent.source.startsWith('magnet:') ? t('Asking the people sharing it for the file list…') : t('Reading the torrent…');
    try {
      info = await api.inspectTorrent(torrent.source);
      picked = new Set(info.files.map((f) => f.index));
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = '';
    }
  });

  function toggle(i: number) {
    const next = new Set(picked);
    next.has(i) ? next.delete(i) : next.add(i);
    picked = next;
  }

  async function pickFolder() {
    const chosen = await open({ directory: true, defaultPath: dir || undefined, title: t('Save downloads to') });
    if (typeof chosen === 'string') dir = chosen;
  }

  async function download() {
    if (!info) return;
    busy = t('Starting…');
    try {
      const all = picked.size === info.files.length;
      await api.add({ url: torrent.source, dir, fileName: null, connections: 1, start: true, headers: [], torrentFiles: all ? null : [...picked] });
      app.torrent = null;
      app.toast(t('Started {name}', { name: info.name }));
    } catch (e) {
      error = errorText(e);
      busy = '';
    }
  }

  const close = () => (app.torrent = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="torrent-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <div>
      <h2 id="torrent-title">{info?.name ?? t('Torrent')}</h2>
      {#if info}<span class="sub">{t('{n} files, {size}', { n: info.files.length, size: bytes(info.size) })}</span>{/if}
    </div>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>

  {#if info}
    <div class="list">
      <ScrollArea>
        <div class="rows">
          {#each info.files as f (f.index)}
            <label class="row">
              <input type="checkbox" checked={picked.has(f.index)} onchange={() => toggle(f.index)} />
              <Icon name={kindIcon(f.path)} />
              <span class="name" title={f.path}>{f.path}</span>
              <span class="size num">{bytes(f.size)}</span>
            </label>
          {/each}
        </div>
      </ScrollArea>
    </div>
    <div class="field">
      <label for="torrent-dir">{t('Save to')}</label>
      <div class="dir-row">
        <input id="torrent-dir" class="slot" bind:value={dir} type="text" spellcheck="false" />
        <button class="key sm" type="button" onclick={pickFolder}><Icon name="folder-open" />{t('Change')}</button>
      </div>
    </div>
    <p class="hint">{app.settings?.seed ? t('When it finishes, KlyxDown keeps sharing it for a while, as torrents rely on that. You can stop sharing any time.') : t('Sharing after it finishes is off in Settings.')}</p>
  {/if}

  {#if busy}<span class="busy" role="status"><span class="scan" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i></span>{busy}</span>{/if}
  {#if error}<span class="err" role="alert">{error}</span>{/if}

  <footer>
    <span class="status">{info ? t('{n} picked, {size} to download', { n: picked.size, size: bytes(pickedSize) }) : ''}</span>
    <button class="key sm" type="button" onclick={close}>{t('Cancel')}</button>
    <button class="key sm go" type="button" disabled={!info || !picked.size || !!busy} onclick={download}><Icon name="download-simple" />{t('Download')}</button>
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
    translate: -50% -50%;
    width: min(640px, calc(100% - 32px));
    max-height: calc(100% - 32px);
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    box-shadow: inset 0 1px 0 var(--hi), 0 30px 60px -20px var(--shadow), 0 0 0 1px var(--lo);
  }
  header {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  header > div {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-stretch: 90%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub,
  .hint,
  .status {
    font-size: 12px;
    color: var(--ink-2);
  }
  .hint {
    margin: 0;
  }
  .status {
    flex: 1;
  }
  .list {
    min-height: 0;
    max-height: 300px;
    display: flex;
    flex-direction: column;
  }
  .rows {
    display: grid;
    gap: 2px;
    padding-inline-end: 10px;
  }
  .row {
    display: grid;
    grid-template-columns: auto 18px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    padding: 5px 8px;
    border-radius: 6px;
  }
  .row:hover {
    background: var(--key);
  }
  .row > :global(.ico) {
    color: var(--ink-3);
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .size {
    font-size: 12px;
    color: var(--ink-2);
  }
  .field {
    display: grid;
    gap: 6px;
  }
  label[for] {
    font-size: 11.5px;
    font-weight: 600;
    font-stretch: 85%;
    color: var(--ink-2);
  }
  .dir-row {
    display: flex;
    gap: 8px;
  }
  .dir-row .slot {
    flex: 1;
    min-width: 0;
  }
  .busy {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--ink-2);
  }
  .err {
    font-size: 12px;
    color: var(--warn);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
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
