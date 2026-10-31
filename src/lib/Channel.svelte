<script lang="ts">
  import { t } from './i18n.svelte';
  import Icon from './Icon.svelte';
  import Leds from './Leds.svelte';
  import Select from './Select.svelte';
  import { api } from './api';
  import { PLAYABLE, bytes, eta, etaText, kindIcon, mbps, pct, speed } from './format';
  import { animate, gsap } from './motion';
  import { app } from './store.svelte';
  import type { DownloadView } from './types';

  let { d, open, ontoggle }: { d: DownloadView; open: boolean; ontoggle: () => void } = $props();

  let flash: HTMLSpanElement;
  const live = $derived(d.state === 'downloading' ? d.segments.filter((s) => s.pos < s.end) : []);
  const finishing = $derived(app.justFinished.has(d.id));

  // Finish moment: the map is already full; the row flashes twice, then leaves the deck.
  $effect(() => {
    if (!finishing) return;
    if (!animate()) return void app.clearFinished(d.id);
    const tl = gsap
      .timeline({ onComplete: () => app.clearFinished(d.id) })
      .to(flash, { opacity: 0.22, duration: 0.15, yoyo: true, repeat: 3 })
      .to({}, { duration: 0.35 });
    return () => tl.kill();
  });

  // Torrents and videos can be watched while they download; conversions can't.
  const canPreview = $derived(d.kind !== 'convert' && (d.kind === 'torrent' || d.kind === 'media' || PLAYABLE.test(d.name)) && d.bytes > 0);

  const meta = $derived.by(() => {
    if (d.detail && d.state === 'downloading') return t(d.detail);
    switch (d.state) {
      case 'downloading':
        if (d.kind === 'torrent') return t('{n} people sharing, {done} of {size}', { n: d.peers, done: bytes(d.bytes), size: bytes(d.size) });
        return t('{host}, {done} of {size}', { host: d.host || t('Web page'), done: bytes(d.bytes), size: bytes(d.size) });
      case 'waitingForNetwork':
        return d.error ? t(d.error) : t('Waiting for the network. It resumes on its own.');
      case 'failed':
        return d.error ? t(d.error) : t('The download stopped.');
      case 'done':
        return t('{size} saved', { size: bytes(d.size) });
      default:
        return d.host;
    }
  });

  function copyLink() {
    navigator.clipboard.writeText(d.url).then(
      () => app.toast(t('Link copied')),
      () => app.toast(t('Could not copy the link')),
    );
  }
</script>

<div
  class="ch"
  class:open
  data-state={d.state}
  role="button"
  tabindex="0"
  aria-expanded={open}
  onclick={(e) => !(e.target as HTMLElement).closest('button, .more') && ontoggle()}
  onkeydown={(e) => e.target === e.currentTarget && (e.key === 'Enter' ? ontoggle() : e.key === ' ' && (e.preventDefault(), d.state === 'downloading' ? app.run(() => api.pause(d.id)) : null))}
>
  <span class="flash" bind:this={flash}></span>
  <div class="conns" aria-hidden="true" title={t('Connections')}>
    {#each Array(8) as _, i (i)}<i class:on={i < live.length}></i>{/each}
  </div>
  <div class="name">
    <b title={d.name}><Icon name={kindIcon(d.name)} size={14} />{d.name}</b>
    <span>{meta}</span>
  </div>
  <div class="map"><Leds {d} /></div>
  <div class="ro">
    {#if d.state === 'downloading' && d.kind === 'convert'}
      <b class="num">{pct(d).toFixed(0)}<small>%</small></b>
      <span>{t('Converting')}</span>
    {:else if d.state === 'downloading'}
      <b class="num">{mbps(d.speed)}<small>{t('MB/s')}</small></b>
      <span class="num">{etaText(eta(d))}</span>
    {:else}
      <b class="status">{d.state === 'waitingForNetwork' ? t('Waiting') : d.state === 'failed' ? t('Failed') : t('Done')}</b>
      <span class="num">{pct(d).toFixed(0)}%</span>
    {/if}
  </div>
  <div class="acts">
    {#if d.state === 'downloading' || d.state === 'waitingForNetwork'}
      <button class="key sm sq" type="button" aria-label={t('Pause')} title={t('Pause')} onclick={() => app.run(() => api.pause(d.id))}><Icon name="pause" /></button>
    {:else if d.state === 'failed' && d.needsLink}
      <button class="key sm sq" type="button" aria-label={t('Get a fresh link')} title={t('Get a fresh link')} onclick={() => (app.refresh = d)}><Icon name="link-break" /></button>
    {:else if d.state === 'failed'}
      <button class="key sm sq" type="button" aria-label={t('Try again')} title={t('Try again')} onclick={() => app.run(() => api.resume(d.id))}><Icon name="arrow-clockwise" /></button>
    {/if}
  </div>

  {#if open}
    <div class="more">
      {#if live.length}
        <div class="lanes" aria-label={t('Connections')}>
          {#each live as s (s.start)}
            <div class="lane"><Leds {d} segment={s} thin /><span class="num">{speed(s.speed)}</span></div>
          {/each}
        </div>
        <p class="note">{t('One lane per connection. When a lane finishes, it takes over half of the biggest part left.')}</p>
      {/if}
      <div class="facts">
        <span>{t('Saved to')} <b>{d.path}</b></span>
        <span>{t('Size')} <b>{bytes(d.size)}</b></span>
        <span>{t('Resumable')} <b>{d.resumable ? t('Yes') : t('No, pausing restarts it')}</b></span>
        {#if d.mirrors}<span>{t('Mirrors')} <b class="num">{d.mirrors}</b></span>{/if}
        {#if d.kind === 'torrent'}
          <span>{t('Sharing')} <b class="num">{speed(d.uploadSpeed)}</b></span>
          <span>{t('Shared so far')} <b class="num">{bytes(d.uploaded)}</b></span>
        {/if}
      </div>
      {#if app.queues.length > 1}
        <div class="queue">
          <span>{t('Queue')}</span>
          <Select id="queue-{d.id}" label={t('Queue')} value={d.queue} options={app.queues.map((q) => ({ value: q.id, label: app.queueName(q.id) }))} onchange={(q) => app.run(() => api.moveToQueue(d.id, q))} />
        </div>
      {/if}
      <div class="row-acts">
        {#if canPreview}
          <button class="key sm go" type="button" onclick={() => (app.preview = d)}><Icon name="play" />{t('Watch while it downloads')}</button>
        {/if}
        <button class="key sm" type="button" onclick={() => app.run(() => api.revealFile(d.id))}><Icon name="folder-open" />{t('Show in folder')}</button>
        <button class="key sm" type="button" onclick={copyLink}><Icon name="copy" />{t('Copy link')}</button>
        {#if d.state !== 'done' && d.kind === 'direct'}
          <button class="key sm" type="button" onclick={() => (app.refresh = d)}><Icon name="link-break" />{t('Get a fresh link')}</button>
        {/if}
        {#if d.state === 'failed'}
          <button class="key sm" type="button" title={t('Throw away the progress and download it again')} onclick={() => app.run(() => api.restart(d.id))}><Icon name="arrow-clockwise" />{t('Restart from zero')}</button>
        {/if}
        <button class="key sm" type="button" onclick={() => app.run(() => api.remove(d.id))}><Icon name="trash" />{t('Remove from list')}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .ch {
    position: relative;
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) minmax(140px, 210px) 96px 28px;
    column-gap: 14px;
    align-items: center;
    padding: 10px 8px 10px 10px;
    border-radius: 6px;
    cursor: default;
  }
  .ch:hover,
  .ch.open {
    background: var(--key);
  }
  .ch.open {
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo);
  }
  .flash {
    position: absolute;
    inset: 0;
    border-radius: 6px;
    background: var(--accent);
    opacity: 0;
    pointer-events: none;
  }
  .conns {
    display: grid;
    grid-template-columns: repeat(2, 5px);
    gap: 3px;
    justify-content: center;
  }
  .conns i {
    width: 5px;
    height: 5px;
    border-radius: 1px;
    background: var(--led-off);
    transition: background-color 0.2s;
  }
  .conns i.on {
    background: var(--accent);
  }
  .name {
    min-width: 0;
  }
  .name b {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name b :global(.ico) {
    color: var(--ink-3);
  }
  .name span {
    display: block;
    margin-top: 1px;
    font-size: 12px;
    color: var(--ink-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  [data-state='failed'] .name span,
  [data-state='failed'] .status {
    color: var(--warn);
  }
  .ro {
    display: grid;
    justify-items: end;
    line-height: 1.15;
  }
  .ro b {
    font-size: 20px;
    font-weight: 650;
    font-stretch: 66%;
  }
  .ro b small {
    margin-inline-start: 2px;
    font-size: 11px;
    font-weight: 600;
    font-stretch: 85%;
    color: var(--ink-2);
  }
  .ro b.status {
    font-size: 13px;
    font-stretch: 85%;
  }
  .ro span {
    font-size: 11.5px;
    color: var(--ink-2);
    white-space: nowrap;
  }
  .more {
    grid-column: 2 / -1;
    display: grid;
    gap: 12px;
    padding: 14px 0 4px;
  }
  .lanes {
    display: grid;
    gap: 4px;
  }
  .lane {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 72px;
    gap: 12px;
    align-items: center;
  }
  .lane span {
    text-align: end;
    font-size: 11.5px;
    color: var(--ink-2);
  }
  .note {
    margin: 0;
    font-size: 12px;
    color: var(--ink-2);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 20px;
    font-size: 12px;
    color: var(--ink-2);
  }
  .facts b {
    color: var(--ink);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .queue {
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: 320px;
    font-size: 12px;
    color: var(--ink-2);
  }
  .row-acts {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
