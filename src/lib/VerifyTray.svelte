<script lang="ts">
  import { t } from './i18n.svelte';
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import { api, errorText } from './api';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { Algo, DownloadView } from './types';

  // Shows the SHA-256 of a finished file; a pasted MD5, SHA-1 or SHA-512 is compared with
  // that algorithm instead (picked from its prefix or length).
  let { d }: { d: DownloadView } = $props();

  const NAMES: Record<Algo, string> = { md5: 'MD5', sha1: 'SHA-1', sha256: 'SHA-256', sha512: 'SHA-512' };
  const LENGTHS: Record<number, Algo> = { 32: 'md5', 40: 'sha1', 64: 'sha256', 128: 'sha512' };
  let hashes = $state<Partial<Record<Algo, string>>>({});
  let error = $state('');
  let expected = $state('');

  const cleaned = $derived(expected.trim().toLowerCase().replace(/^(md5|sha-?1|sha-?256|sha-?512)[:=\s]*/, ''));
  const algo = $derived<Algo>(LENGTHS[cleaned.length] ?? 'sha256');
  const hash = $derived(hashes[algo] ?? '');
  const verdict = $derived(!cleaned || !hash ? null : cleaned === hash ? 'match' : 'mismatch');

  async function compute(a: Algo) {
    if (hashes[a]) return;
    try {
      const value = await api.fileHash(d.id, a);
      hashes = { ...hashes, [a]: value };
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(() => compute('sha256'));
  $effect(() => void compute(algo));

  function copy() {
    navigator.clipboard.writeText(hash).then(
      () => app.toast(t('Checksum copied')),
      () => app.toast(t('Could not copy the checksum')),
    );
  }

  const close = () => (app.verify = null);
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="tray panel" role="dialog" aria-modal="true" aria-labelledby="verify-title" transition:fly={{ y: -14, duration: ms(260) }}>
  <header>
    <h2 id="verify-title">{t('Check the file')}</h2>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>
  <p class="file" title={d.path}>{d.name}</p>

  <div class="field">
    <span class="label">{t('{algo} of the file on disk', { algo: NAMES[algo] })}</span>
    {#if error}
      <span class="err" role="alert">{error}</span>
    {:else if !hash}
      <span class="scan" aria-label={t('Reading the file')}><i></i><i></i><i></i><i></i><i></i><i></i></span>
    {:else}
      <div class="hash-row">
        <code class="hash">{hash}</code>
        <button class="key sm sq" type="button" aria-label={t('Copy checksum')} title={t('Copy')} onclick={copy}><Icon name="copy" /></button>
      </div>
    {/if}
  </div>

  <div class="field">
    <label class="label" for="verify-expected">{t('Checksum from the website')}</label>
    <input id="verify-expected" class="slot" bind:value={expected} type="text" placeholder={t('Paste it here to compare')} spellcheck="false" autocomplete="off" />
    {#if verdict === 'match'}
      <span class="ok" role="status"><Icon name="check" size={14} /> {t('Matches. The file is exactly what the website published.')}</span>
    {:else if verdict === 'mismatch'}
      <span class="err" role="status"><Icon name="warning-circle" size={14} /> {t('Does not match. Download it again before opening it.')}</span>
    {/if}
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
    /* `translate` composes with the fly transition's `transform`, so the tray stays centered while it animates. */
    translate: -50% -50%;
    width: min(560px, calc(100% - 32px));
    max-height: calc(100% - 32px);
    padding: 16px;
    display: grid;
    gap: 14px;
    box-shadow: inset 0 1px 0 var(--hi), 0 30px 60px -20px var(--shadow), 0 0 0 1px var(--lo);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 15px;
    font-stretch: 90%;
  }
  .file {
    margin: -6px 0 0;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .field {
    display: grid;
    gap: 6px;
  }
  .hash-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .hash {
    flex: 1;
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
    font: 12px ui-monospace, Consolas, monospace;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .ok,
  .err {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
  }
  .ok {
    color: var(--accent);
  }
  .err {
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
    0%, 60%, 100% { background: var(--led-off); }
    20% { background: var(--accent); }
  }
</style>
