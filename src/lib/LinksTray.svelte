<script lang="ts">
  import { msg, t } from './i18n.svelte';
  import { untrack } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import ScrollArea from './ScrollArea.svelte';
  import { api, errorText } from './api';
  import { kindIcon } from './format';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { CapturedLinks } from './types';

  // "Download all links" from the browser: pick which links to download.
  let { links }: { links: CapturedLinks } = $props();

  const GROUPS: [string, RegExp][] = [
    [msg('Archives'), /\.(zip|7z|rar|gz|xz|tar|bz2|zst)$/i],
    [msg('Programs'), /\.(exe|msi|msix|pkg|dmg|deb|rpm|appimage|apk)$/i],
    [msg('Video'), /\.(mp4|mkv|webm|mov|avi|m4v)$/i],
    [msg('Audio'), /\.(mp3|flac|wav|ogg|m4a|aac|opus)$/i],
    [msg('Images'), /\.(jpe?g|png|gif|webp|avif|svg|bmp|tiff?)$/i],
    [msg('Documents'), /\.(pdf|epub|docx?|xlsx?|pptx?|txt|csv|odt)$/i],
    [msg('Disk images'), /\.(iso|img|vhdx?)$/i],
  ];

  const OTHER = msg('Pages and other');

  interface Item {
    url: string;
    name: string;
    host: string;
    group: string;
  }

  // A new batch remounts this component (App wraps it in {#key}), so read the list once.
  const items: Item[] = untrack(() => links.urls).map((url) => {
    let name = url;
    let host = '';
    try {
      const u = new URL(url);
      host = u.host;
      name = decodeURIComponent(u.pathname.split('/').filter(Boolean).pop() ?? '') || u.host;
    } catch {}
    const group = GROUPS.find(([, re]) => re.test(name))?.[0] ?? OTHER;
    return { url, name, host, group };
  });

  // Files are picked by default; plain page links are not.
  let picked = $state(new Set(items.filter((i) => i.group !== OTHER).map((i) => i.url)));
  let query = $state('');
  let group = $state<string | null>(null);
  let busy = $state('');

  const groups = $derived(
    [...new Set(items.map((i) => i.group))].map((g) => ({ name: g, count: items.filter((i) => i.group === g).length })),
  );
  const shown = $derived(
    items.filter(
      (i) => (!group || i.group === group) && (!query || (i.name + ' ' + i.url).toLowerCase().includes(query.toLowerCase())),
    ),
  );
  const pickedCount = $derived(picked.size);

  function toggle(url: string) {
    const next = new Set(picked);
    next.has(url) ? next.delete(url) : next.add(url);
    picked = next;
  }
  function setShown(on: boolean) {
    const next = new Set(picked);
    shown.forEach((i) => (on ? next.add(i.url) : next.delete(i.url)));
    picked = next;
  }

  async function download() {
    const urls = items.filter((i) => picked.has(i.url)).map((i) => i.url);
    let failed = 0;
    for (const [n, url] of urls.entries()) {
      busy = t('Adding {done} of {n}…', { done: n + 1, n: urls.length });
      try {
        await api.add({
          url,
          dir: '',
          fileName: null,
          connections: app.settings?.connections ?? 8,
          start: true,
          headers: links.headers,
        });
      } catch (e) {
        failed++;
        if (failed === 1) app.toast(errorText(e));
      }
    }
    busy = '';
    app.links = null;
    app.toast(failed ? t('Added {done} of {n}. {failed} could not be added.', { done: urls.length - failed, n: urls.length, failed }) : t('Added {n} downloads', { n: urls.length }));
  }

  const close = () => (app.links = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="links-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <div>
      <h2 id="links-title">{t('{browser} found {n} links', { browser: links.browser, n: items.length })}</h2>
      {#if links.title}<span class="sub">{links.title}</span>{/if}
    </div>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>

  <div class="tools">
    <input class="slot" bind:value={query} type="text" placeholder={t('Filter by name, for example .zip')} aria-label={t('Filter links')} spellcheck="false" />
    <div class="chips" role="group" aria-label={t('File types')}>
      <button class="key sm" type="button" aria-pressed={group === null} onclick={() => (group = null)}>{t('All')} <span class="n num">{items.length}</span></button>
      {#each groups as g (g.name)}
        <button class="key sm" type="button" aria-pressed={group === g.name} onclick={() => (group = group === g.name ? null : g.name)}>
          {t(g.name)} <span class="n num">{g.count}</span>
        </button>
      {/each}
    </div>
  </div>

  <div class="list">
    <ScrollArea>
      <div class="rows">
        {#each shown as item (item.url)}
          <label class="row">
            <input type="checkbox" checked={picked.has(item.url)} onchange={() => toggle(item.url)} />
            <Icon name={kindIcon(item.name)} />
            <span class="name" title={item.url}>{item.name}</span>
            <span class="host">{item.host}</span>
          </label>
        {:else}
          <p class="empty">{t('No links match.')}</p>
        {/each}
      </div>
    </ScrollArea>
  </div>

  <footer>
    <button class="key sm" type="button" onclick={() => setShown(true)}>{t('Select shown')}</button>
    <button class="key sm" type="button" onclick={() => setShown(false)}>{t('Clear shown')}</button>
    <span class="status" role="status">{busy || t('{n} picked', { n: pickedCount })}</span>
    <button class="key sm" type="button" onclick={close}>{t('Cancel')}</button>
    <button class="key sm go" type="button" disabled={!pickedCount || !!busy} onclick={download}>
      {t('Download {n}', { n: pickedCount || '' })}
    </button>
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
    width: min(680px, calc(100% - 32px));
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
  }
  .sub {
    display: block;
    font-size: 12px;
    color: var(--ink-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tools {
    display: grid;
    gap: 8px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .n {
    color: var(--ink-2);
    font-weight: 500;
  }
  .list {
    flex: 1;
    min-height: 120px;
    max-height: 340px;
    display: flex;
    flex-direction: column;
    border-radius: 6px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
  }
  .list :global(.area) {
    flex: 1;
  }
  .rows {
    padding: 4px 12px 4px 4px;
  }
  .row {
    display: grid;
    grid-template-columns: 18px 16px minmax(0, 1fr) minmax(0, 0.6fr);
    gap: 10px;
    align-items: center;
    padding: 6px 8px;
    border-radius: 4px;
    cursor: pointer;
  }
  .row:hover {
    background: var(--panel);
  }
  .row :global(.ico) {
    color: var(--ink-3);
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 600;
  }
  .host {
    font-size: 12px;
    color: var(--ink-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  input[type='checkbox'] {
    appearance: none;
    width: 16px;
    height: 16px;
    margin: 0;
    border-radius: 4px;
    background: var(--key);
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo);
    display: grid;
    place-items: center;
    cursor: pointer;
  }
  input[type='checkbox']:checked {
    background: var(--accent);
  }
  input[type='checkbox']:checked::after {
    content: '';
    width: 8px;
    height: 4px;
    margin-top: -2px;
    border-inline-start: 2px solid var(--on-accent);
    border-bottom: 2px solid var(--on-accent);
    transform: rotate(-45deg);
  }
  .empty {
    margin: 0;
    padding: 20px;
    text-align: center;
    color: var(--ink-2);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .status {
    flex: 1;
    font-size: 12px;
    color: var(--ink-2);
  }
</style>
