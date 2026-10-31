<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import ScrollArea from './ScrollArea.svelte';
  import { api, errorText } from './api';
  import { bytes, kindIcon } from './format';
  import { t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { FolderEntry } from './types';

  // Site explorer: browse a web server's folder listing or an FTP server like a file
  // browser, and pick files to download.
  let { start }: { start: { url: string } } = $props();

  // The panel opens from one link; typing changes it from there.
  let url = $state(untrack(() => start.url));
  let entries = $state<FolderEntry[] | null>(null);
  let picked = $state(new Map<string, FolderEntry>());
  let busy = $state(false);
  let error = $state('');

  const parent = $derived.by(() => {
    try {
      const u = new URL(url);
      if (u.pathname === '/' || u.pathname === '') return null;
      return new URL(u.pathname.endsWith('/') ? '..' : '.', u).toString();
    } catch {
      return null;
    }
  });

  onMount(() => {
    if (url.trim()) go(url);
  });

  async function go(to: string) {
    busy = true;
    error = '';
    try {
      entries = await api.listFolder(to.trim());
      url = to.trim();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  function toggle(e: FolderEntry) {
    const next = new Map(picked);
    next.has(e.url) ? next.delete(e.url) : next.set(e.url, e);
    picked = next;
  }

  function download() {
    const urls = [...picked.keys()];
    app.explore = null;
    app.links = { urls, title: url, headers: [], browser: t('Site explorer') };
  }

  const close = () => (app.explore = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="explore-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <h2 id="explore-title">{t('Browse a site')}</h2>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>
  <form class="bar" onsubmit={(e) => (e.preventDefault(), go(url))}>
    <button class="key sm sq" type="button" aria-label={t('Up one folder')} title={t('Up one folder')} disabled={!parent || busy} onclick={() => parent && go(parent)}><Icon name="arrow-up" /></button>
    <input class="slot" bind:value={url} type="text" placeholder={t('https:// or ftp:// folder')} aria-label={t('Folder address')} spellcheck="false" autocomplete="off" />
    <button class="key sm" type="submit" disabled={busy}>{t('Open')}</button>
  </form>

  <div class="list">
    <ScrollArea>
      <div class="rows">
        {#if entries}
          {#each entries as e (e.url)}
            {#if e.dir}
              <button class="row dir" type="button" onclick={() => go(e.url)}>
                <span></span>
                <Icon name="folder" />
                <span class="name">{e.name}</span>
                <Icon name="caret-right" size={12} />
              </button>
            {:else}
              <label class="row">
                <input type="checkbox" checked={picked.has(e.url)} onchange={() => toggle(e)} />
                <Icon name={kindIcon(e.name)} />
                <span class="name" title={e.url}>{e.name}</span>
                <span class="size num">{e.size === null ? '' : bytes(e.size)}</span>
              </label>
            {/if}
          {:else}
            <p class="empty">{t('This folder is empty.')}</p>
          {/each}
        {:else if !busy && !error}
          <p class="empty">{t('Paste the address of a folder: a web server listing (Index of /…) or an FTP server.')}</p>
        {/if}
      </div>
    </ScrollArea>
  </div>
  {#if error}<span class="err" role="alert">{error}</span>{/if}

  <footer>
    <span class="status">{busy ? t('Opening…') : picked.size ? t('{n} picked', { n: picked.size }) : ''}</span>
    <button class="key sm" type="button" onclick={close}>{t('Cancel')}</button>
    <button class="key sm go" type="button" disabled={!picked.size} onclick={download}>{t('Download {n}', { n: picked.size || '' })}</button>
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
    height: min(560px, calc(100% - 32px));
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    box-shadow: inset 0 1px 0 var(--hi), 0 30px 60px -20px var(--shadow), 0 0 0 1px var(--lo);
  }
  header {
    display: flex;
    align-items: center;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 15px;
    font-stretch: 90%;
  }
  .bar {
    display: flex;
    gap: 8px;
  }
  .bar .slot {
    flex: 1;
    min-width: 0;
  }
  .list {
    flex: 1;
    min-height: 0;
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
    grid-template-columns: 16px 18px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    padding: 5px 8px;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--ink);
    font: inherit;
    text-align: start;
  }
  .row:hover {
    background: var(--key);
  }
  .row :global(.ico) {
    color: var(--ink-3);
  }
  .dir :global(.ico:first-of-type) {
    color: var(--accent);
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .size,
  .status {
    font-size: 12px;
    color: var(--ink-2);
  }
  .status {
    flex: 1;
  }
  .empty {
    margin: 0;
    padding: 24px 16px;
    text-align: center;
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
</style>
