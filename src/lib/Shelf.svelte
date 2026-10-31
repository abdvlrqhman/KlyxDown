<script lang="ts">
  import { t } from './i18n.svelte';
  import { slide } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import ScrollArea from './ScrollArea.svelte';
  import { api } from './api';
  import { PLAYABLE, bytes, finishedText, kindIcon, pct } from './format';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { DownloadView } from './types';

  type Tab = 'latest' | 'waiting' | 'paused' | 'finished';
  let tab = $state<Tab | null>('latest');

  // Newest first everywhere: last finished, or last added for the rest.
  const newest = (list: DownloadView[]) => [...list].sort((a, b) => (b.finishedAt ?? b.addedAt) - (a.finishedAt ?? a.addedAt));
  const LATEST = 50;
  const tabs = $derived([
    { key: 'latest' as const, label: t('Latest'), items: newest([...app.waiting, ...app.paused, ...app.finished]).slice(0, LATEST) },
    { key: 'waiting' as const, label: t('Waiting'), items: newest(app.waiting) },
    { key: 'paused' as const, label: t('Paused'), items: newest(app.paused) },
    { key: 'finished' as const, label: t('Finished'), items: newest(app.finished) },
  ]);
  const items = $derived(tabs.find((t) => t.key === tab)?.items ?? []);

  function status(d: DownloadView) {
    if (d.state === 'queued') return d.queue ? t('Waiting in {queue}', { queue: app.queueName(d.queue) }) : t('Starts when a slot frees up');
    if (d.state === 'paused') return t('Paused at {pct}%', { pct: pct(d).toFixed(0) });
    if (d.seeding) return t('Sharing, {size} shared so far', { size: bytes(d.uploaded) });
    return finishedText(d.finishedAt);
  }
</script>

<div class="shelf">
  <div class="tabs" role="tablist" aria-label={t('Other downloads')}>
    {#each tabs as t (t.key)}
      <button
        class="key sm"
        role="tab"
        type="button"
        aria-selected={tab === t.key}
        onclick={() => (tab = tab === t.key ? null : t.key)}
      >
        {t.label} <span class="n num">{t.items.length}</span>
      </button>
    {/each}
  </div>

  {#if tab}
    <div class="body" role="tabpanel" transition:slide={{ duration: ms(260) }}>
      <ScrollArea><div class="rows">
      {#each items as d (d.id)}
        <div class="row" data-state={d.state}>
          <Icon name={kindIcon(d.name)} />
          <b title={d.name}>{d.name}</b>
          <span class="host">{d.host}</span>
          <span class="size num">{bytes(d.size)}</span>
          <span class="status">{status(d)}</span>
          <span class="acts">
            {#if d.state === 'done'}
              <button class="key sm sq" type="button" aria-label={t('Open')} title={t('Open')} onclick={() => app.run(() => api.openFile(d.id))}><Icon name="arrow-square-out" /></button>
              <button class="key sm sq" type="button" aria-label={t('Show in folder')} title={t('Show in folder')} onclick={() => app.run(() => api.revealFile(d.id))}><Icon name="folder-open" /></button>
              {#if d.kind !== 'torrent'}
                <button class="key sm sq" type="button" aria-label={t('Check the file')} title={t('Check the file (SHA-256)')} onclick={() => (app.verify = d)}><Icon name="check" /></button>
              {/if}
              {#if PLAYABLE.test(d.name) || /\.(ts|avi|flv|wmv)$/i.test(d.name)}
                <button class="key sm sq" type="button" aria-label={t('Convert')} title={t('Convert to another format')} onclick={() => (app.convert = d)}><Icon name="arrows-clockwise" /></button>
              {/if}
              {#if d.kind === 'torrent'}
                <button class="key sm sq" type="button" aria-label={d.seeding ? t('Stop sharing') : t('Share again')} title={d.seeding ? t('Stop sharing') : t('Share again')} aria-pressed={d.seeding} onclick={() => app.run(() => api.setSeeding(d.id, !d.seeding))}><Icon name="upload-simple" /></button>
              {/if}
            {:else}
              <button class="key sm sq" type="button" aria-label={t('Start')} title={t('Start')} onclick={() => app.run(() => api.resume(d.id))}><Icon name="play" /></button>
            {/if}
            <button class="key sm sq" type="button" aria-label={t('Remove from list')} title={t('Remove from list')} onclick={() => app.run(() => api.remove(d.id))}><Icon name="x" /></button>
          </span>
        </div>
      {:else}
        <p class="empty">{tab === 'latest' ? t('Nothing here yet. Finished, paused and waiting downloads show up here.') : t('Nothing here right now.')}</p>
      {/each}
      </div></ScrollArea>
    </div>
  {/if}
</div>

<style>
  .shelf {
    margin-top: auto;
    padding: 8px;
    display: grid;
    gap: 6px;
  }
  .tabs {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .tabs .key[aria-selected='true'] {
    transform: translateY(1px);
    background: var(--panel);
    box-shadow: inset 0 1px 2px var(--lo);
  }
  .n {
    color: var(--ink-2);
    font-weight: 500;
  }
  .body {
    max-height: 220px;
    display: flex;
    flex-direction: column;
  }
  .body :global(.area) {
    flex: 1 1 auto;
  }
  .rows {
    display: grid;
    gap: 2px;
    padding-inline-end: 10px;
  }
  .row {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) minmax(0, 0.7fr) 80px minmax(0, 1fr) auto;
    gap: 12px;
    align-items: center;
    padding: 6px 8px;
    border-radius: 6px;
  }
  .row:hover {
    background: var(--key);
  }
  .row > :global(.ico) {
    color: var(--ink-3);
  }
  .row b {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .host,
  .status {
    font-size: 12px;
    color: var(--ink-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .size {
    text-align: end;
    font-size: 12.5px;
  }
  .acts {
    display: flex;
    gap: 4px;
  }
  .empty {
    margin: 0;
    padding: 16px;
    text-align: center;
    color: var(--ink-2);
  }
</style>
