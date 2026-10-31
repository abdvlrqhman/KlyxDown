<script lang="ts">
  import { t } from './i18n.svelte';
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { open } from '@tauri-apps/plugin-dialog';
  import Icon from './Icon.svelte';
  import Select from './Select.svelte';
  import { api, errorText } from './api';
  import { bytes } from './format';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { CapturedLinks, Checksum, Probe } from './types';

  const ALGOS = { 32: 'md5', 40: 'sha1', 64: 'sha256', 128: 'sha512' } as const;
  function parseChecksum(text: string): Checksum {
    const hex = text.replace(/^(md5|sha-?1|sha-?256|sha-?512)[:=\s]*/i, '').toLowerCase();
    return { algo: ALGOS[hex.length as keyof typeof ALGOS], hex };
  }

  // Downloads caught by the browser extension carry its cookies and referrer.
  const capture = app.capture;
  let url = $state(app.addPrefill);
  let found = $state<Probe | null>(null);
  let name = $state('');
  let dir = $state(app.settings?.downloadDir ?? '');
  let dirPicked = false;
  let connections = $state(app.settings?.connections ?? 8);
  let queue = $state(0);
  const queues = $derived(app.queues.map((q) => ({ value: q.id, label: app.queueName(q.id) })));
  let busy = $state(false);
  let error = $state('');
  let input: HTMLInputElement;
  // Optional: other links to the same file, and the checksum the site publishes.
  let more = $state(false);
  let mirrors = $state('');
  let checksum = $state('');
  let convert = $state('');
  const CONVERSIONS = $derived([
    { value: '', label: t('Keep as it is') },
    { value: 'mp4', label: t('MP4 video') },
    { value: 'mp4-copy', label: t('MP4, same quality (fast)') },
    { value: 'mp3', label: t('MP3 audio') },
    { value: 'm4a', label: t('M4A audio') },
  ]);
  const playable = $derived(!!found && /\.(mp4|mkv|webm|mov|avi|m4v|ts|flv|wmv|mp3|m4a|aac|ogg|opus|flac|wav)$/i.test(name));
  // A link to a page, not a file: offer to look for the video on it.
  const page = $derived(!!found?.mime?.startsWith('text/html'));
  const VIDEO_SITES = /(^|\.)(youtube\.com|youtu\.be|vimeo\.com|dailymotion\.com|twitch\.tv|tiktok\.com|instagram\.com|facebook\.com|fb\.watch|x\.com|twitter\.com|reddit\.com|soundcloud\.com|bandcamp\.com|bilibili\.com|rumble\.com|streamable\.com)$/i;
  const isZip = $derived(!!found?.resumable && found.size !== null && /\.zip$/i.test(found.name));
  const CONNECTIONS = [1, 2, 4, 8, 16, 32].map((n) => ({ value: n, label: t('{n} connections', { n }) }));

  onMount(() => {
    if (url) check();
    else input.focus();
  });

  async function check() {
    const link = url.trim();
    if (!link) return void (error = t('Paste a link first.'));
    error = '';
    busy = true;
    try {
      if (/^magnet:/i.test(link) || /\.torrent([?#]|$)/i.test(link)) return openTorrent(link);
      if (VIDEO_SITES.test(hostOf(link))) return openMedia(link);
      // photo[001-120].jpg stands for 120 links: pick from them instead.
      const batch = await api.expandBatch(link);
      if (batch.length) return showLinks({ urls: batch, title: link, headers: capture?.headers ?? [], browser: t('Batch link') });
      if (/\.(meta4|metalink)([?#]|$)/i.test(link)) {
        const files = await api.readMetalink(link);
        return showLinks({ urls: files.map((f) => f.urls[0]!), files, title: link, headers: [], browser: t('Metalink') });
      }
      found = await api.probe(link, capture?.headers ?? []);
      name = capture?.name ?? found.name;
      if (!dirPicked) dir = await api.suggestDir(name);
    } catch (e) {
      found = null;
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  function hostOf(link: string) {
    try {
      return new URL(link).hostname;
    } catch {
      return '';
    }
  }

  function openTorrent(source: string) {
    app.closeAdd(true);
    app.torrent = { source };
  }

  function openMedia(link: string) {
    const headers = capture?.headers ?? [];
    // A browser hand-off is answered here: KlyxDown takes it, as a video.
    if (capture?.id) api.captureDone(capture.id, true).catch(() => {});
    app.closeAdd(true);
    app.media = { url: link, headers };
  }

  async function pickTorrentFile() {
    const path = await open({ title: t('Open a .torrent file'), filters: [{ name: t('Torrent files'), extensions: ['torrent'] }] });
    if (typeof path === 'string') openTorrent(path);
  }

  function showLinks(links: CapturedLinks) {
    app.closeAdd();
    app.links = links;
  }

  async function add(start: boolean) {
    if (!found) return;
    const sum = checksum.trim();
    if (sum && !/^((md5|sha-?1|sha-?256|sha-?512)[:=\s]*)?([0-9a-f]{32}|[0-9a-f]{40}|[0-9a-f]{64}|[0-9a-f]{128})$/i.test(sum)) {
      error = t('That checksum is not MD5, SHA-1, SHA-256 or SHA-512.');
      return;
    }
    busy = true;
    try {
      await api.add({
        url: found.url,
        dir,
        fileName: name.trim() && name.trim() !== found.name ? name.trim() : null,
        connections: found.resumable ? connections : 1,
        start,
        headers: capture?.headers ?? [],
        queue,
        mirrors: mirrors.split(/\s+/).filter((m) => /^(https?|ftps?|ftpes):\/\//i.test(m)),
        checksum: sum ? parseChecksum(sum) : null,
        convert: convert || null,
      });
      if (capture?.id) await api.captureDone(capture.id, true);
      app.closeAdd(true);
      app.toast(start ? t('Started {name}', { name }) : t('Added {name}, paused', { name }));
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true, defaultPath: dir || undefined, title: t('Save downloads to') });
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
    <h2 id="add-title">{capture ? t('{browser} sent a download', { browser: capture.browser }) : t('Add a download')}</h2>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>

  <form class="field" onsubmit={(e) => (e.preventDefault(), found ? add(true) : check())}>
    <label for="add-url">{t('Link')}</label>
    <div class="url-row">
      <input id="add-url" class="slot" bind:this={input} bind:value={url} type="text" placeholder={t('https://')} autocomplete="off" spellcheck="false" oninput={() => (found = null)} />
      <button class="key" type="submit" disabled={busy}>{found ? t('Check again') : t('Check link')}</button>
    </div>
    {#if busy}<span class="scan" aria-label={t('Checking')}><i></i><i></i><i></i><i></i><i></i><i></i></span>{/if}
    {#if error}<span class="err" role="alert">{error}</span>{/if}
  </form>

  {#if found}
    <div class="found" in:fly={{ y: 6, duration: ms(220) }}>
      <div class="field">
        <label for="add-name">{t('File name')}</label>
        <div class="name-row">
          <input id="add-name" class="slot" bind:value={name} type="text" spellcheck="false" />
          <span class="size num">{bytes(found.size)}</span>
        </div>
      </div>
      <div class="two">
        <div class="field">
          <label for="add-dir">{t('Save to')}</label>
          <div class="dir-row">
            <input id="add-dir" class="slot" bind:value={dir} type="text" spellcheck="false" />
            <button class="key sm" type="button" onclick={pickFolder}><Icon name="folder-open" />{t('Change')}</button>
          </div>
        </div>
        <div class="field">
          <label for="add-conns">{t('Connections')}</label>
          <Select id="add-conns" label={t('Connections')} bind:value={connections} disabled={!found.resumable} options={CONNECTIONS} />
        </div>
      </div>
      {#if queues.length > 1}
        <div class="field queue">
          <label for="add-queue">{t('Queue')}</label>
          <Select id="add-queue" label={t('Queue')} bind:value={queue} options={queues} />
        </div>
      {/if}
      {#if page}
        <div class="page-hint">
          <Icon name="globe" />
          <span>{t('This link is a web page, not a file. KlyxDown can look for the video on it.')}</span>
          <button class="key sm" type="button" onclick={() => openMedia(url.trim())}><Icon name="video-camera" />{t('Get the video')}</button>
        </div>
      {/if}
      <button class="more-toggle" type="button" aria-expanded={more} onclick={() => (more = !more)}>
        <Icon name="caret-right" size={12} />{playable ? t('Mirrors, checksum and conversion') : t('Mirrors and checksum')}
      </button>
      {#if more}
        <div class="two" transition:fly={{ y: -4, duration: ms(160) }}>
          <div class="field">
            <label for="add-mirrors">{t('Mirrors, one link per line')}</label>
            <textarea id="add-mirrors" class="slot" rows="2" bind:value={mirrors} spellcheck="false" placeholder="https://"></textarea>
          </div>
          <div class="field">
            <label for="add-sum">{t('Checksum from the site')}</label>
            <input id="add-sum" class="slot" bind:value={checksum} type="text" spellcheck="false" placeholder="SHA-256" />
          </div>
        </div>
        {#if playable}
          <div class="field convert">
            <label for="add-convert">{t('When it finishes, convert it to')}</label>
            <Select id="add-convert" label={t('Convert')} bind:value={convert} options={CONVERSIONS} />
          </div>
        {/if}
      {/if}
      <p class="hint">
        {found.resumable
          ? t('The server supports resuming, so pausing is safe and several connections can share the work.')
          : t('This server cannot resume. It downloads on one connection, and pausing restarts it.')}
      </p>
    </div>
  {/if}

  <footer>
    {#if !found}
      <button class="key sm torrent-file" type="button" onclick={pickTorrentFile}><Icon name="magnet" />{t('Open a .torrent file')}</button>
    {/if}
    {#if isZip && found}
      <button class="key sm zip" type="button" disabled={busy} onclick={() => found && (app.zip = { url: found.url, name: found.name, headers: capture?.headers ?? [] }, app.closeAdd())}>
        <Icon name="file-zip" />{t('Pick files inside')}
      </button>
    {/if}
    <button class="key sm" type="button" onclick={close}>{capture?.id ? t('Let {browser} download it', { browser: capture.browser }) : t('Cancel')}</button>
    <button class="key sm" type="button" disabled={!found || busy} onclick={() => add(false)}>{t('Add paused')}</button>
    <button class="key sm go" type="button" disabled={!found || busy} onclick={() => add(true)}>{t('Start download')}</button>
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
  .queue {
    max-width: 260px;
  }
  .zip,
  .torrent-file {
    margin-inline-end: auto;
  }
  .convert {
    max-width: 280px;
  }
  .page-hint {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--chassis);
    font-size: 12px;
    color: var(--ink-2);
  }
  .page-hint span {
    flex: 1;
  }
  .page-hint :global(.ico) {
    color: var(--accent);
  }
  .more-toggle {
    justify-self: start;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--ink-2);
    font: 600 12px var(--font);
  }
  .more-toggle :global(.ico) {
    transition: transform 0.15s var(--ease);
  }
  .more-toggle[aria-expanded='true'] :global(.ico) {
    transform: rotate(90deg);
  }
  textarea.slot {
    resize: vertical;
    font: inherit;
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
