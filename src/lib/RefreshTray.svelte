<script lang="ts">
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import { api, errorText } from './api';
  import { t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { DownloadView } from './types';

  // A link that expired (or worked only once): get a fresh one for the same file and keep
  // the progress. Either from the page, like IDM does, or pasted.
  let { d }: { d: DownloadView } = $props();

  let url = $state('');
  let busy = $state(false);
  let error = $state('');
  let waiting = $state(false);

  async function fromPage() {
    error = '';
    try {
      await api.waitForLink(d.id);
      waiting = true;
    } catch (e) {
      error = errorText(e);
    }
  }

  async function paste(e: SubmitEvent) {
    e.preventDefault();
    if (!url.trim()) return;
    busy = true;
    error = '';
    try {
      await api.refreshLink(d.id, url.trim());
      app.refresh = null;
      app.toast(t('Fresh link accepted. {name} continues where it stopped.', { name: d.name }));
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  const close = () => (app.refresh = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="refresh-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <h2 id="refresh-title">{t('Get a fresh link')}</h2>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>
  <p class="file" title={d.url}>{d.name}</p>
  <p class="hint">{t('The link stopped working, which happens with links that expire or only work once. A new link to the same file continues the download without losing what is already saved.')}</p>

  {#if d.page}
    <div class="step">
      {#if waiting}
        <span class="ok" role="status"><Icon name="globe" size={14} />{t('The page is open in your browser. Click the download link on it again; KlyxDown picks it up and continues.')}</span>
      {:else}
        <button class="key sm go" type="button" onclick={fromPage}><Icon name="globe" />{t('Open the page and click the link again')}</button>
      {/if}
    </div>
  {/if}

  <form class="paste" onsubmit={paste}>
    <label for="fresh-url">{d.page ? t('Or paste a fresh link') : t('Paste a fresh link')}</label>
    <div class="row">
      <input id="fresh-url" class="slot" bind:value={url} type="text" placeholder="https://" spellcheck="false" autocomplete="off" />
      <button class="key sm" type="submit" disabled={busy || !url.trim()}>{t('Use this link')}</button>
    </div>
  </form>
  {#if error}<span class="err" role="alert">{error}</span>{/if}
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
    width: min(540px, calc(100% - 32px));
    max-height: calc(100% - 32px);
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
  .file {
    margin: 0;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--ink-2);
  }
  .ok {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    font-size: 12.5px;
    color: var(--accent);
  }
  .paste {
    display: grid;
    gap: 6px;
  }
  label {
    font-size: 11.5px;
    font-weight: 600;
    font-stretch: 85%;
    color: var(--ink-2);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .row .slot {
    flex: 1;
    min-width: 0;
  }
  .err {
    font-size: 12px;
    color: var(--warn);
  }
</style>
