<script lang="ts">
  import { t } from './lib/i18n.svelte';
  import { clock } from './lib/format';
  import { onMount, tick } from 'svelte';
  import { flip } from 'svelte/animate';
  import { fade, slide } from 'svelte/transition';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import AddTray from './lib/AddTray.svelte';
  import Channel from './lib/Channel.svelte';
  import Icon from './lib/Icon.svelte';
  import Countdown from './lib/Countdown.svelte';
  import Intro from './lib/Intro.svelte';
  import LinksTray from './lib/LinksTray.svelte';
  import Onboarding from './lib/Onboarding.svelte';
  import Queues from './lib/Queues.svelte';
  import RefreshTray from './lib/RefreshTray.svelte';
  import ZipTray from './lib/ZipTray.svelte';
  import MediaTray from './lib/MediaTray.svelte';
  import TorrentTray from './lib/TorrentTray.svelte';
  import PreviewTray from './lib/PreviewTray.svelte';
  import ConvertTray from './lib/ConvertTray.svelte';
  import GrabTray from './lib/GrabTray.svelte';
  import ExploreTray from './lib/ExploreTray.svelte';
  import VerifyTray from './lib/VerifyTray.svelte';
  import Logo from './lib/Logo.svelte';
  import Master from './lib/Master.svelte';
  import ScrollArea from './lib/ScrollArea.svelte';
  import Settings from './lib/Settings.svelte';
  import Shelf from './lib/Shelf.svelte';
  import TitleBar from './lib/TitleBar.svelte';
  import Toasts from './lib/Toasts.svelte';
  import UpdateBar from './lib/UpdateBar.svelte';
  import { api, errorText } from './lib/api';
  import { animate, gsap, ms } from './lib/motion';
  import { app } from './lib/store.svelte';
  import { applyTheme, onSystemThemeChange } from './lib/themes';
  import type { Captured, CapturedLinks, CapturedMedia, DownloadView, FinishAction } from './lib/types';

  let openId = $state<number | null>(null);
  let shell: HTMLDivElement;

  // Rows stay on the deck while their finish moment plays.
  const deck = $derived(app.downloads.filter((d) => app.running.includes(d) || app.justFinished.has(d.id)));

  onMount(() => {
    const cleanups: (() => void)[] = [];
    (async () => {
      try {
        await app.loadSettings();
        await app.loadQueues();
        await api.watch((s) => app.setSnapshot(s));
      } catch (e) {
        app.toast(errorText(e));
      }
      // Only now, with the UI painted, may the window appear (never a blank or error page).
      const start = await api.uiReady().catch(() => ({ show: true, capture: null, links: null, media: null }));
      if (start.capture) app.openAdd(start.capture.url, start.capture);
      if (start.links) app.links = start.links;
      if (start.media) app.takeMedia(start.media);
      if (start.show) {
        // The intro covers the app before the window appears, so the first frame is the intro.
        if (app.settings?.showIntro) {
          app.intro = true;
          await tick();
        }
        const win = getCurrentWindow();
        await win.unminimize();
        await win.show();
        await win.setFocus();
      }
      if (!app.intro) afterIntro();
    })();

    // Failures get a toast; everything else shows up through the progress stream.
    listen<DownloadView>('download:changed', ({ payload }) => {
      if (payload.state === 'failed') app.toast(payload.error ? t(payload.error) : t('{name} stopped.', { name: payload.name }));
    }).then((off) => cleanups.push(off));
    // The browser extension hands over downloads and page links (src-tauri/src/bridge.rs).
    listen<Captured>('capture', ({ payload }) => app.openAdd(payload.url, payload)).then((off) => cleanups.push(off));
    listen<CapturedLinks>('capture-links', ({ payload }) => (app.links = payload)).then((off) => cleanups.push(off));
    listen<CapturedMedia>('capture-media', ({ payload }) => app.takeMedia(payload)).then((off) => cleanups.push(off));
    listen<string>('notice', ({ payload }) => app.toast(t(payload))).then((off) => cleanups.push(off));
    listen<number>('link-refreshed', ({ payload }) => {
      const d = app.downloads.find((x) => x.id === payload);
      if (app.refresh?.id === payload) app.refresh = null;
      app.toast(t('Fresh link received. {name} continues where it stopped.', { name: d?.name ?? '' }));
    }).then((off) => cleanups.push(off));
    listen<[number, number | null]>('update-progress', ({ payload }) => (app.updating = { received: payload[0], total: payload[1] })).then((off) => cleanups.push(off));
    listen<FinishAction>('finish-countdown', ({ payload }) => {
      app.finishAction = 'nothing';
      app.countdown = payload;
    }).then((off) => cleanups.push(off));
    listen<number>('capture-expired', ({ payload }) => {
      if (app.capture?.id !== payload) return;
      app.capture = null;
      app.addOpen = false;
      app.toast(t('No answer for a while, so the browser kept that download.'));
    }).then((off) => cleanups.push(off));

    // Network status: the webview hears about it first; tell the engine.
    const setOnline = (online: boolean) => {
      const wasOnline = app.online;
      app.online = online;
      api.networkChanged(online).catch(() => {});
      if (online && !wasOnline) app.toast(t('Back online. Downloads are resuming.'));
    };
    const goOnline = () => setOnline(true);
    const goOffline = () => setOnline(false);
    addEventListener('online', goOnline);
    addEventListener('offline', goOffline);
    api.networkChanged(navigator.onLine).catch(() => {});
    cleanups.push(() => {
      removeEventListener('online', goOnline);
      removeEventListener('offline', goOffline);
    });

    cleanups.push(onSystemThemeChange(() => app.settings?.theme === 'system' && applyTheme('system')));
    return () => cleanups.forEach((f) => f());
  });

  // The tray menu and tooltip follow the interface language.
  $effect(() => {
    api
      .setTrayText({
        show: t('Show KlyxDown'),
        pauseAll: t('Pause all'),
        resumeAll: t('Resume all'),
        quit: t('Quit KlyxDown'),
        idle: t('KlyxDown: nothing downloading'),
        busy: t('KlyxDown: {n} downloading at {speed} MB/s'),
        offline: t("KlyxDown: offline, downloads resume when you're back"),
      })
      .catch(() => {});
  });

  let search: HTMLInputElement;

  function afterIntro() {
    app.intro = false;
    powerOn();
    if (app.settings && !app.settings.onboarded) app.onboarding = true;
    // A quiet check a few seconds after launch; no message unless there is something new.
    if (app.settings?.autoUpdate) setTimeout(() => api.checkUpdate().then((u) => u && (app.update = u)).catch(() => {}), 4000);
  }

  // One orchestrated moment on launch: panels settle, then every LED sweeps once.
  function powerOn() {
    if (!animate()) return;
    const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
    gsap
      .timeline()
      .from(shell.querySelectorAll(':scope > .panel, :scope > header'), { y: 10, autoAlpha: 0, stagger: 0.06, duration: 0.4, ease: 'power3.out', clearProps: 'all' })
      .fromTo(
        shell.querySelectorAll('.leds i, .vu i'),
        { backgroundColor: accent },
        { backgroundColor: getComputedStyle(document.documentElement).getPropertyValue('--led-off').trim(), duration: 0.25, stagger: { each: 0.002 }, clearProps: 'backgroundColor' },
        0.15,
      );
  }

  function onkeydown(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    const typing = (e.target as HTMLElement).closest('input, textarea, [contenteditable]');
    if ((mod && e.key.toLowerCase() === 'f') || (e.key === '/' && !typing)) {
      e.preventDefault();
      search.focus();
      search.select();
    } else if (mod && e.key.toLowerCase() === 'n') {
      e.preventDefault();
      app.openAdd();
    } else if (mod && e.key === ',') {
      e.preventDefault();
      app.settingsOpen = !app.settingsOpen;
    } else if (e.key === 'Escape') {
      if (e.target === search && app.query) app.query = '';
      else if (app.addOpen) app.closeAdd();
      else if (app.links) app.links = null;
      else if (app.preview) app.preview = null;
      else if (app.convert) app.convert = null;
      else if (app.grab) app.grab = null;
      else if (app.explore) app.explore = null;
      else if (app.zip) app.zip = null;
      else if (app.media) app.media = null;
      else if (app.torrent) app.torrent = null;
      else if (app.refresh) app.refresh = null;
      else if (app.verify) app.verify = null;
      else if (app.settingsOpen) app.settingsOpen = false;
      else if (app.queuesOpen) app.queuesOpen = false;
    }
  }

  // Links dragged from a browser onto the window. Native file drop is off
  // (dragDropEnabled: false in tauri.conf.json) so the page receives the link.
  let dropping = $state(false);
  const linksIn = (e: DragEvent) =>
    (e.dataTransfer?.getData('text/uri-list') || e.dataTransfer?.getData('text/plain') || '')
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => /^(https?:|ftps?:|ftpes:|magnet:)/i.test(l));
  function ondragover(e: DragEvent) {
    if (!e.dataTransfer?.types.some((t) => t === 'text/uri-list' || t === 'text/plain')) return;
    e.preventDefault();
    dropping = true;
  }
  function ondragleave(e: DragEvent) {
    if (!e.relatedTarget) dropping = false;
  }
  function ondrop(e: DragEvent) {
    e.preventDefault();
    dropping = false;
    const urls = linksIn(e);
    if (urls.length === 1) app.openAdd(urls[0]);
    else if (urls.length > 1) app.links = { urls, title: null, headers: [], browser: t('Dropped links') };
  }

  // Pasting a link anywhere (outside a text field) opens the add panel with it.
  function onpaste(e: ClipboardEvent) {
    if ((e.target as HTMLElement).closest('input, textarea')) return;
    const text = e.clipboardData?.getData('text').trim() ?? '';
    if (/^(https?:|ftps?:|ftpes:|magnet:)/i.test(text)) app.openAdd(text);
  }
</script>

<svelte:window {onkeydown} {onpaste} {ondragover} {ondragleave} {ondrop} />

<div class="shell" bind:this={shell}>
  <TitleBar />

  {#if app.update}<UpdateBar update={app.update} />{/if}

  {#if !app.online}
    <div class="offline" role="status" transition:slide={{ duration: ms(220) }}>
      <Icon name="wifi-slash" />
      <span>{t('You\'re offline. Downloads wait and continue on their own when the connection is back.')}</span>
    </div>
  {:else if app.status.hold}
    {@const hold = app.status.hold}
    <div class="offline" role="status" transition:slide={{ duration: ms(220) }}>
      <Icon name={hold.kind === 'metered' ? 'wifi-slash' : 'calendar-blank'} />
      <span>
        {hold.kind === 'metered'
          ? t('Downloads are on hold because this connection is metered.')
          : t('The download quota is used up. Downloads continue at {time}.', { time: clock(hold.resumesAt) })}
      </span>
      {#if hold.kind === 'metered'}
        <button class="key sm" type="button" onclick={() => app.run(api.allowMetered)}>{t('Download anyway')}</button>
      {:else}
        <button class="key sm" type="button" onclick={() => (app.settingsOpen = true)}>{t('Change the quota')}</button>
      {/if}
    </div>
  {/if}

  <section class="deck panel" aria-label={t('Downloads')}>
    {#if app.needsYou.length}
      <div class="head"><span class="label warn">{t('Needs you')}</span><span class="label num">{app.needsYou.length}</span></div>
      <div class="rows">
        {#each app.needsYou as d (d.id)}
          <div animate:flip={{ duration: ms(300) }} transition:slide={{ duration: ms(240) }}>
            <Channel {d} open={openId === d.id} ontoggle={() => (openId = openId === d.id ? null : d.id)} />
          </div>
        {/each}
      </div>
    {/if}

    <div class="head">
      <span class="label">{t('Running')}</span><span class="label num">{app.running.length}</span>
      <label class="search slot" title={t('Search downloads (Ctrl F)')}>
        <Icon name="magnifying-glass" size={14} />
        <input bind:this={search} bind:value={app.query} type="search" placeholder={t('Search')} aria-label={t('Search downloads')} spellcheck="false" autocomplete="off" />
      </label>
    </div>
    <ScrollArea class="deck-scroll">
      <div class="rows" class:idle={!deck.length}>
        {#each deck as d (d.id)}
          <div animate:flip={{ duration: ms(300) }} transition:slide={{ duration: ms(260) }}>
            <Channel {d} open={openId === d.id} ontoggle={() => (openId = openId === d.id ? null : d.id)} />
          </div>
        {:else}
          <div class="empty" in:fade={{ duration: ms(200) }}>
            <Logo size={44} />
            {#if app.query}
              <p>{t('Nothing running matches “{query}”.', { query: app.query })}</p>
              <p class="hint">{t('Waiting, paused and finished matches are in the shelf below.')}</p>
            {:else}
            <p>{t('Nothing is downloading.')}</p>
            <p class="hint">{#each t('Paste a link above, or anywhere in this window. {keys} opens the add panel.').split('{keys}') as part, i (i)}{#if i}<kbd>Ctrl</kbd> <kbd>N</kbd>{/if}{part}{/each}</p>
            {/if}
          </div>
        {/each}
      </div>
    </ScrollArea>

    <Shelf />
  </section>

  <Master />
</div>

{#if app.addOpen}{#key app.addKey}<AddTray />{/key}{/if}
{#if app.links}{#key app.links}<LinksTray links={app.links} />{/key}{/if}
{#if app.verify}<VerifyTray d={app.verify} />{/if}
{#if app.zip}<ZipTray zip={app.zip} />{/if}
{#if app.refresh}<RefreshTray d={app.refresh} />{/if}
{#if app.media}<MediaTray media={app.media} />{/if}
{#if app.torrent}<TorrentTray torrent={app.torrent} />{/if}
{#if app.preview}<PreviewTray d={app.preview} />{/if}
{#if app.convert}<ConvertTray d={app.convert} />{/if}
{#if app.grab}<GrabTray start={app.grab} />{/if}
{#if app.explore}<ExploreTray start={app.explore} />{/if}
{#if app.onboarding}<Onboarding />{/if}
{#if app.intro}<Intro ondone={afterIntro} />{/if}
{#if app.countdown}<Countdown action={app.countdown} />{/if}
{#if dropping}<div class="drop" transition:fade={{ duration: ms(120) }}><div class="drop-panel"><Icon name="tray-arrow-down" size={28} /><span>{t('Drop to download')}</span></div></div>{/if}
{#if app.settingsOpen}<Settings />{/if}
{#if app.queuesOpen}<Queues />{/if}
<Toasts />

<style>
  .shell {
    height: 100%;
    display: grid;
    gap: 8px;
    padding: 8px;
    grid-template-columns: minmax(0, 1fr) 240px;
    grid-template-rows: auto auto minmax(0, 1fr);
    grid-template-areas: 'top top' 'banner banner' 'deck master';
  }
  .offline {
    grid-area: banner;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--warn) 16%, var(--panel));
    color: var(--ink);
    font-size: 12.5px;
  }
  .offline :global(.ico) {
    color: var(--warn);
  }
  .offline span {
    flex: 1;
  }
  .search {
    margin-inline-start: auto;
    display: flex;
    align-items: center;
    gap: 6px;
    width: 200px;
    height: 28px;
    padding-block: 0;
    color: var(--ink-3);
  }
  .search:focus-within {
    color: var(--ink-2);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--ink);
    font: inherit;
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  .deck {
    grid-area: deck;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .deck :global(.deck-scroll) {
    flex: 1;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px 8px;
  }
  .label.warn {
    color: var(--warn);
  }
  .rows {
    display: grid;
    gap: 4px;
    padding: 0 12px 0 8px;
  }
  /* With nothing running, the empty note sits in the middle of the deck. */
  .rows.idle {
    min-height: 100%;
    align-content: center;
  }
  .empty {
    display: grid;
    justify-items: center;
    gap: 4px;
    padding: 48px 16px;
    text-align: center;
    color: var(--ink-2);
  }
  .empty p {
    margin: 0;
  }
  .empty p:first-of-type {
    margin-top: 10px;
    color: var(--ink);
    font-weight: 600;
  }
  .hint {
    font-size: 12px;
  }
  .drop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--recess) 60%, transparent);
    pointer-events: none;
  }
  .drop-panel {
    display: grid;
    justify-items: center;
    gap: 8px;
    padding: 28px 40px;
    border-radius: 10px;
    background: var(--panel);
    color: var(--accent);
    font-weight: 650;
    box-shadow: 0 0 0 2px var(--accent), 0 30px 60px -20px var(--shadow);
  }
  .drop-panel span {
    color: var(--ink);
  }
  kbd {
    font: 600 11px var(--font);
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--key);
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo);
  }
</style>
