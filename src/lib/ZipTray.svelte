<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import ScrollArea from './ScrollArea.svelte';
  import { api, errorText } from './api';
  import { bytes, kindIcon } from './format';
  import { t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { ZipEntry } from './types';

  // Files inside a remote ZIP: pick some and download only those, not the whole archive.
  let { zip }: { zip: { url: string; name: string; headers: [string, string][] } } = $props();

  let entries = $state<ZipEntry[] | null>(null);
  let error = $state('');
  let picked = $state(new Set<string>());
  let query = $state('');
  let busy = $state('');

  const files = $derived((entries ?? []).filter((e) => !e.dir));
  const shown = $derived(files.filter((e) => !query || e.name.toLowerCase().includes(query.toLowerCase())));
  const total = $derived(files.reduce((n, e) => n + e.size, 0));
  const pickedSize = $derived(files.filter((e) => picked.has(e.name)).reduce((n, e) => n + e.compressed, 0));

  onMount(async () => {
    try {
      entries = await api.listZip(zip.url, zip.headers);
    } catch (e) {
      error = errorText(e);
    }
  });

  function toggle(name: string) {
    const next = new Set(picked);
    next.has(name) ? next.delete(name) : next.add(name);
    picked = next;
  }

  const why = (e: ZipEntry) => (e.encrypted ? t('Password-protected') : e.method !== 0 && e.method !== 8 ? t('Unsupported compression') : '');

  async function download() {
    const chosen = files.filter((e) => picked.has(e.name));
    let failed = 0;
    for (const [i, e] of chosen.entries()) {
      busy = t('Adding {done} of {n}…', { done: i + 1, n: chosen.length });
      try {
        await api.add({ url: zip.url, dir: '', fileName: null, connections: app.settings?.connections ?? 8, start: true, headers: zip.headers, zipEntry: e.name });
      } catch (err) {
        if (!failed++) app.toast(errorText(err));
      }
    }
    app.zip = null;
    app.toast(t('Added {n} downloads', { n: chosen.length - failed }));
  }

  const close = () => (app.zip = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="zip-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <div>
      <h2 id="zip-title">{t('Inside {name}', { name: zip.name })}</h2>
      {#if entries}<span class="sub">{t('{n} files, {size} unpacked. Only the files you pick are downloaded.', { n: files.length, size: bytes(total) })}</span>{/if}
    </div>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>

  {#if error}
    <p class="err" role="alert">{error}</p>
  {:else if !entries}
    <span class="scan" aria-label={t('Reading the archive')}><i></i><i></i><i></i><i></i><i></i><i></i></span>
  {:else}
    <input class="slot" bind:value={query} type="text" placeholder={t('Filter by name')} aria-label={t('Filter files')} spellcheck="false" />
    <div class="list">
      <ScrollArea>
        <div class="rows">
          {#each shown as e (e.name)}
            {@const reason = why(e)}
            <label class="row" class:off={!!reason}>
              <input type="checkbox" checked={picked.has(e.name)} disabled={!!reason} onchange={() => toggle(e.name)} />
              <Icon name={kindIcon(e.name)} />
              <span class="name" title={e.name}>{e.name}</span>
              <span class="size num">{reason || bytes(e.size)}</span>
            </label>
          {:else}
            <p class="empty">{t('No files match.')}</p>
          {/each}
        </div>
      </ScrollArea>
    </div>
  {/if}

  <footer>
    <span class="status" role="status">{busy || (picked.size ? t('{n} picked, {size} to download', { n: picked.size, size: bytes(pickedSize) }) : '')}</span>
    <button class="key sm" type="button" onclick={close}>{t('Cancel')}</button>
    <button class="key sm go" type="button" disabled={!picked.size || !!busy} onclick={download}>{t('Download {n}', { n: picked.size || '' })}</button>
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
  .sub {
    font-size: 12px;
    color: var(--ink-2);
  }
  .list {
    min-height: 0;
    max-height: 340px;
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
  .row.off {
    opacity: 0.55;
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
  .empty {
    margin: 0;
    padding: 16px;
    text-align: center;
    color: var(--ink-2);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status {
    flex: 1;
    font-size: 12px;
    color: var(--ink-2);
  }
  .err {
    margin: 0;
    font-size: 12.5px;
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
