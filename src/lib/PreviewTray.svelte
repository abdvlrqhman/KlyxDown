<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, scale } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import { api, errorText } from './api';
  import { t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { DownloadView } from './types';

  // Plays a download while it downloads. The engine fetches what the player asks for first,
  // so seeking works too (src-tauri/src/preview.rs).
  let { d }: { d: DownloadView } = $props();

  let src = $state('');
  let error = $state('');
  const audio = $derived(/\.(mp3|m4a|aac|ogg|opus|flac|wav)$/i.test(d.name));

  onMount(async () => {
    try {
      src = await api.previewUrl(d.id);
    } catch (e) {
      error = errorText(e);
    }
  });

  const close = () => (app.preview = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="player panel" role="dialog" aria-modal="true" aria-label={t('Preview of {name}', { name: d.name })} transition:scale={{ start: 0.97, duration: ms(220) }}>
  <header>
    <b title={d.name}>{d.name}</b>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>
  {#if src}
    {#if audio}
      <!-- svelte-ignore a11y_media_has_caption -->
      <audio {src} controls autoplay onerror={() => (error = t('This file can not be played here. It plays once it is complete, in your own player.'))}></audio>
    {:else}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video {src} controls autoplay onerror={() => (error = t('This file can not be played here. It plays once it is complete, in your own player.'))}></video>
    {/if}
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  {#if d.state !== 'done'}<p class="hint">{t('Still downloading. Jump ahead and KlyxDown fetches that part next.')}</p>{/if}
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: color-mix(in srgb, var(--recess) 75%, transparent);
  }
  .player {
    position: fixed;
    z-index: 41;
    top: 50%;
    left: 50%;
    translate: -50% -50%;
    width: min(900px, calc(100% - 32px));
    max-height: calc(100% - 32px);
    padding: 12px;
    display: grid;
    gap: 10px;
    box-shadow: inset 0 1px 0 var(--hi), 0 30px 60px -20px var(--shadow), 0 0 0 1px var(--lo);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  header b {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  video {
    width: 100%;
    max-height: calc(100vh - 160px);
    border-radius: 6px;
    background: #000;
  }
  audio {
    width: 100%;
  }
  .hint,
  .err {
    margin: 0;
    font-size: 12px;
    color: var(--ink-2);
  }
  .err {
    color: var(--warn);
  }
</style>
