<script lang="ts">
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import { api } from './api';
  import { msg, t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { IconName } from './icons';
  import type { DownloadView } from './types';

  // Converts a finished file with ffmpeg. The original stays; the result lands next to it.
  let { d }: { d: DownloadView } = $props();

  const PRESETS: { id: string; label: string; note: string; icon: IconName }[] = [
    { id: 'mp4', label: msg('MP4 video'), note: msg('Plays everywhere. Takes a while: the video is encoded again.'), icon: 'film-strip' },
    { id: 'mp4-copy', label: msg('MP4, same quality'), note: msg('Seconds, no quality loss. Works when the streams already suit MP4 (most TS and MKV files).'), icon: 'film-strip' },
    { id: 'mkv-copy', label: msg('MKV, same quality'), note: msg('Seconds, keeps every track and subtitle.'), icon: 'film-strip' },
    { id: 'webm', label: msg('WebM video'), note: msg('Small files for the web. Slow to make.'), icon: 'film-strip' },
    { id: 'mp3', label: msg('MP3 audio'), note: msg('Just the sound, for any player.'), icon: 'music-note' },
    { id: 'm4a', label: msg('M4A audio'), note: msg('Just the sound, smaller than MP3 at the same quality.'), icon: 'music-note' },
  ];

  async function go(preset: string) {
    await app.run(async () => {
      await api.convert(d.id, preset);
      app.toast(t('Converting {name}', { name: d.name }));
    });
    app.convert = null;
  }

  const close = () => (app.convert = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="convert-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <div>
      <h2 id="convert-title">{t('Convert')}</h2>
      <span class="sub" title={d.path}>{d.name}</span>
    </div>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>
  <div class="grid">
    {#each PRESETS as p (p.id)}
      <button class="preset" type="button" onclick={() => go(p.id)}>
        <Icon name={p.icon} size={18} />
        <b>{t(p.label)}</b>
        <span>{t(p.note)}</span>
      </button>
    {/each}
  </div>
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
    width: min(560px, calc(100% - 32px));
    max-height: calc(100% - 32px);
    overflow: auto;
    padding: 16px;
    display: grid;
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
    display: grid;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-stretch: 90%;
  }
  .sub {
    font-size: 12px;
    color: var(--ink-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  .preset {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 2px 10px;
    padding: 10px;
    border: 0;
    border-radius: 8px;
    background: var(--key);
    color: var(--ink);
    text-align: start;
    font: inherit;
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo);
  }
  .preset:hover {
    background: var(--key-hover);
  }
  .preset :global(.ico) {
    grid-row: span 2;
    color: var(--accent);
  }
  .preset span {
    font-size: 11.5px;
    color: var(--ink-2);
  }
</style>
