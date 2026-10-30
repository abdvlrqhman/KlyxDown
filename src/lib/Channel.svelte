<script lang="ts">
  import Icon from './Icon.svelte';
  import Leds from './Leds.svelte';
  import { api } from './api';
  import { bytes, eta, etaText, kindIcon, mbps, pct, speed } from './format';
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

  const meta = $derived.by(() => {
    switch (d.state) {
      case 'downloading':
        return `${d.host}, ${bytes(d.bytes)} of ${bytes(d.size)}`;
      case 'waitingForNetwork':
        return d.error ?? 'Waiting for the network. It resumes on its own.';
      case 'failed':
        return d.error ?? 'The download stopped.';
      case 'done':
        return `${bytes(d.size)} saved`;
      default:
        return d.host;
    }
  });

  function copyLink() {
    navigator.clipboard.writeText(d.url).then(
      () => app.toast('Link copied'),
      () => app.toast('Could not copy the link'),
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
  <div class="conns" aria-hidden="true" title="Connections">
    {#each Array(8) as _, i (i)}<i class:on={i < live.length}></i>{/each}
  </div>
  <div class="name">
    <b title={d.name}><Icon name={kindIcon(d.name)} size={14} />{d.name}</b>
    <span>{meta}</span>
  </div>
  <div class="map"><Leds {d} /></div>
  <div class="ro">
    {#if d.state === 'downloading'}
      <b class="num">{mbps(d.speed)}<small>MB/s</small></b>
      <span class="num">{etaText(eta(d))}</span>
    {:else}
      <b class="status">{d.state === 'waitingForNetwork' ? 'Waiting' : d.state === 'failed' ? 'Failed' : 'Done'}</b>
      <span class="num">{pct(d).toFixed(0)}%</span>
    {/if}
  </div>
  <div class="acts">
    {#if d.state === 'downloading' || d.state === 'waitingForNetwork'}
      <button class="key sm sq" type="button" aria-label="Pause" title="Pause" onclick={() => app.run(() => api.pause(d.id))}><Icon name="pause" /></button>
    {:else if d.state === 'failed'}
      <button class="key sm sq" type="button" aria-label="Try again" title="Try again" onclick={() => app.run(() => api.resume(d.id))}><Icon name="arrow-clockwise" /></button>
    {/if}
  </div>

  {#if open}
    <div class="more">
      {#if live.length}
        <div class="lanes" aria-label="Connections">
          {#each live as s (s.start)}
            <div class="lane"><Leds {d} segment={s} thin /><span class="num">{speed(s.speed)}</span></div>
          {/each}
        </div>
        <p class="note">One lane per connection. When a lane finishes, it takes over half of the biggest part left.</p>
      {/if}
      <div class="facts">
        <span>Saved to <b>{d.path}</b></span>
        <span>Size <b>{bytes(d.size)}</b></span>
        <span>Resumable <b>{d.resumable ? 'Yes' : 'No, pausing restarts it'}</b></span>
      </div>
      <div class="row-acts">
        <button class="key sm" type="button" onclick={() => app.run(() => api.revealFile(d.id))}><Icon name="folder-open" />Show in folder</button>
        <button class="key sm" type="button" onclick={copyLink}><Icon name="copy" />Copy link</button>
        {#if d.state === 'failed'}
          <button class="key sm" type="button" title="Throw away the progress and download it again" onclick={() => app.run(() => api.restart(d.id))}><Icon name="arrow-clockwise" />Restart from zero</button>
        {/if}
        <button class="key sm" type="button" onclick={() => app.run(() => api.remove(d.id))}><Icon name="trash" />Remove from list</button>
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
    margin-left: 2px;
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
    text-align: right;
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
  .row-acts {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
