<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { listen } from '@tauri-apps/api/event';
  import Icon from './Icon.svelte';
  import { api, errorText } from './api';
  import { msg, t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';

  // Site grabber: read a site's pages from a starting page and collect links to files of the
  // picked types, then pick from them in the links panel.
  let { start }: { start: { url: string } } = $props();

  const TYPES: { label: string; ext: string[] }[] = [
    { label: msg('Images'), ext: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'svg', 'bmp', 'tif', 'tiff'] },
    { label: msg('Video'), ext: ['mp4', 'mkv', 'webm', 'mov', 'avi', 'm4v'] },
    { label: msg('Audio'), ext: ['mp3', 'flac', 'wav', 'ogg', 'm4a', 'aac', 'opus'] },
    { label: msg('Documents'), ext: ['pdf', 'epub', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'csv', 'odt'] },
    { label: msg('Archives'), ext: ['zip', '7z', 'rar', 'gz', 'xz', 'tar', 'bz2', 'zst'] },
    { label: msg('Programs'), ext: ['exe', 'msi', 'msix', 'pkg', 'dmg', 'deb', 'rpm', 'appimage', 'apk'] },
  ];

  // The panel opens from one link; typing changes it from there.
  let url = $state(untrack(() => start.url));
  let picked = $state(new Set([TYPES[0]!.label]));
  let depth = $state(1);
  let sameSite = $state(true);
  let running = $state(false);
  let progress = $state({ pages: 0, found: 0 });
  let error = $state('');

  onMount(() => {
    const off = listen<{ pages: number; found: number }>('grab-progress', ({ payload }) => (progress = payload));
    return () => {
      off.then((f) => f());
      if (running) api.stopGrab().catch(() => {});
    };
  });

  function toggle(label: string) {
    const next = new Set(picked);
    next.has(label) ? next.delete(label) : next.add(label);
    picked = next;
  }

  async function run(e: SubmitEvent) {
    e.preventDefault();
    if (!url.trim()) return;
    running = true;
    error = '';
    progress = { pages: 0, found: 0 };
    try {
      const extensions = TYPES.filter((ty) => picked.has(ty.label)).flatMap((ty) => ty.ext);
      const urls = await api.grab(url.trim(), { extensions, depth, sameSite, maxPages: 300 });
      if (!urls.length) {
        error = t('No matching files found. Try another type, or look one more link deep.');
        return;
      }
      app.grab = null;
      app.links = { urls, title: url.trim(), headers: [], browser: t('Site grabber') };
    } catch (err) {
      error = errorText(err);
    } finally {
      running = false;
    }
  }

  const close = () => (app.grab = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="grab-title" transition:fly={{ y: -14, duration: ms(260) }}>
<form class="body" onsubmit={run}>
  <header>
    <h2 id="grab-title">{t('Find files on a site')}</h2>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>
  <div class="field">
    <label for="grab-url">{t('Start from this page')}</label>
    <input id="grab-url" class="slot" bind:value={url} type="text" placeholder="https://" spellcheck="false" autocomplete="off" disabled={running} />
  </div>
  <div class="field">
    <span class="label">{t('Look for')}</span>
    <div class="chips" role="group" aria-label={t('File types')}>
      {#each TYPES as ty (ty.label)}
        <button class="key sm" type="button" aria-pressed={picked.has(ty.label)} disabled={running} onclick={() => toggle(ty.label)}>{t(ty.label)}</button>
      {/each}
    </div>
  </div>
  <div class="two">
    <div class="field">
      <span class="label">{t('How deep')}</span>
      <div class="stepper">
        <button class="key sm sq" type="button" aria-label={t('Less')} disabled={depth <= 0 || running} onclick={() => depth--}><Icon name="minus" size={14} /></button>
        <output class="num">{depth === 0 ? t('This page only') : t('{n} links away', { n: depth })}</output>
        <button class="key sm sq" type="button" aria-label={t('More')} disabled={depth >= 5 || running} onclick={() => depth++}><Icon name="plus" size={14} /></button>
      </div>
    </div>
    <label class="check"><input type="checkbox" bind:checked={sameSite} disabled={running} />{t('Stay on this site')}</label>
  </div>
  <p class="hint">{t('KlyxDown reads one page at a time and skips what the site asks robots not to read.')}</p>

  {#if running}<span class="status" role="status">{t('{pages} pages read, {found} files found', { pages: progress.pages, found: progress.found })}</span>{/if}
  {#if error}<span class="err" role="alert">{error}</span>{/if}

  <footer>
    {#if running}
      <button class="key sm" type="button" onclick={() => api.stopGrab()}>{t('Stop and show what was found')}</button>
    {:else}
      <button class="key sm" type="button" onclick={close}>{t('Cancel')}</button>
      <button class="key sm go" type="submit" disabled={!picked.size || !url.trim()}><Icon name="magnifying-glass" />{t('Find files')}</button>
    {/if}
  </footer>
</form>
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
    padding: 16px;
    box-shadow: inset 0 1px 0 var(--hi), 0 30px 60px -20px var(--shadow), 0 0 0 1px var(--lo);
  }
  .body {
    display: grid;
    gap: 12px;
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
  .field {
    display: grid;
    gap: 6px;
  }
  label[for],
  .label {
    font-size: 11.5px;
    font-weight: 600;
    font-stretch: 85%;
    color: var(--ink-2);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chips .key[aria-pressed='true'] {
    background: var(--accent);
    color: var(--on-accent);
  }
  .two {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
  }
  .stepper {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .stepper output {
    min-width: 110px;
    text-align: center;
    font-weight: 600;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .hint,
  .status {
    margin: 0;
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
</style>
