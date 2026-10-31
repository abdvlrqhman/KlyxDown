<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import { api, errorText } from './api';
  import { bytes } from './format';
  import { t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { MediaChoice, MediaInfo, ToolStatus } from './types';

  // Video or audio from a web page: pick a quality, KlyxDown runs yt-dlp (and ffmpeg to merge
  // the picture and the sound). The helpers are downloaded once, after asking.
  let { media }: { media: { url: string; headers: [string, string][] } } = $props();

  let tools = $state<ToolStatus | null>(null);
  let info = $state<MediaInfo | null>(null);
  let pick = $state<MediaChoice | null>(null);
  let error = $state('');
  let busy = $state('');

  const ready = $derived(!!tools?.ytdlp);
  const duration = $derived(info?.duration ? `${Math.floor(info.duration / 60)}:${String(Math.round(info.duration % 60)).padStart(2, '0')}` : '');

  onMount(async () => {
    try {
      tools = await api.toolsStatus();
      if (tools.ytdlp) await read();
    } catch (e) {
      error = errorText(e);
    }
  });

  async function install() {
    busy = t('Downloading yt-dlp and ffmpeg…');
    error = '';
    try {
      tools = await api.installTools();
      await read();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = '';
    }
  }

  async function read() {
    busy = t('Reading the page…');
    try {
      info = await api.mediaInfo(media.url, media.headers);
      pick = info.choices.find((c) => !c.audio && (c.height ?? 0) <= 1080) ?? info.choices[0] ?? null;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = '';
    }
  }

  async function download() {
    if (!info || !pick) return;
    busy = t('Starting…');
    try {
      await api.add({ url: media.url, dir: '', fileName: info.title, connections: 1, start: true, headers: media.headers, media: pick });
      app.media = null;
      app.toast(t('Started {name}', { name: info.title }));
    } catch (e) {
      error = errorText(e);
      busy = '';
    }
  }

  const close = () => (app.media = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="media-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <h2 id="media-title">{t('Video from a web page')}</h2>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>
  <p class="link" title={media.url}>{media.url}</p>

  {#if tools && !ready}
    <div class="consent">
      <Icon name="video-camera" size={22} />
      <div>
        <b>{t('KlyxDown needs two free helpers for videos')}</b>
        <p>
          {t('yt-dlp finds the video on the page, and ffmpeg joins its picture and sound. They come from their official GitHub releases (about 20 MB, more with ffmpeg) and are checked before use.')}
          {#if !tools.ffmpegDownloadable && !tools.ffmpeg}{t('On Linux, install ffmpeg from your package manager (for example: sudo apt install ffmpeg).')}{/if}
        </p>
      </div>
    </div>
  {:else if info}
    <div class="found" in:fly={{ y: 6, duration: ms(220) }}>
      <div class="title">
        {#if info.thumbnail}<img src={info.thumbnail} alt="" referrerpolicy="no-referrer" />{/if}
        <div>
          <b>{info.title}</b>
          <span>{info.site}{duration ? ` · ${duration}` : ''}{info.live ? ` · ${t('Live')}` : ''}</span>
        </div>
      </div>
      <div class="choices" role="radiogroup" aria-label={t('Quality')}>
        {#each info.choices as c (c.label)}
          <button class="choice" type="button" role="radio" aria-checked={pick === c} onclick={() => (pick = c)}>
            <Icon name={c.audio ? 'music-note' : 'film-strip'} size={14} />
            <span>{t(c.label)}</span>
            <small class="num">{c.size ? `~${bytes(c.size)}` : ''}</small>
          </button>
        {/each}
      </div>
    </div>
  {/if}

  {#if busy}<span class="busy" role="status"><span class="scan" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i></span>{busy}</span>{/if}
  {#if error}<span class="err" role="alert">{error}</span>{/if}

  <footer>
    <button class="key sm" type="button" onclick={close}>{t('Cancel')}</button>
    {#if tools && !ready}
      <button class="key sm go" type="button" disabled={!!busy} onclick={install}><Icon name="download-simple" />{t('Get the helpers')}</button>
    {:else}
      <button class="key sm go" type="button" disabled={!pick || !!busy} onclick={download}><Icon name="download-simple" />{t('Download')}</button>
    {/if}
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
    width: min(600px, calc(100% - 32px));
    max-height: calc(100% - 32px);
    overflow: auto;
    padding: 16px;
    display: grid;
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
  .link {
    margin: 0;
    font-size: 12px;
    color: var(--ink-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .consent {
    display: flex;
    gap: 12px;
    padding: 12px;
    border-radius: 8px;
    background: var(--chassis);
  }
  .consent :global(.ico) {
    color: var(--accent);
  }
  .consent p {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--ink-2);
  }
  .found {
    display: grid;
    gap: 12px;
  }
  .title {
    display: flex;
    gap: 12px;
    align-items: center;
  }
  .title img {
    width: 96px;
    height: 54px;
    object-fit: cover;
    border-radius: 6px;
    background: var(--recess);
  }
  .title div {
    display: grid;
    min-width: 0;
  }
  .title b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title span {
    font-size: 12px;
    color: var(--ink-2);
  }
  .choices {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 6px;
  }
  .choice {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 0;
    border-radius: 6px;
    background: var(--key);
    color: var(--ink);
    font: 600 12.5px var(--font);
    text-align: start;
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo);
  }
  .choice[aria-checked='true'] {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .choice small {
    font-weight: 500;
    color: var(--ink-2);
  }
  .choice :global(.ico) {
    color: var(--ink-3);
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
    justify-content: flex-end;
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
