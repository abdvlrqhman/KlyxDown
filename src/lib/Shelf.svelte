<script lang="ts">
  import { slide } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import ScrollArea from './ScrollArea.svelte';
  import { api } from './api';
  import { bytes, finishedText, kindIcon, pct } from './format';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { DownloadView } from './types';

  type Tab = 'waiting' | 'paused' | 'finished';
  let tab = $state<Tab | null>(null);

  const tabs = $derived([
    { key: 'waiting' as const, label: 'Waiting', items: app.waiting },
    { key: 'paused' as const, label: 'Paused', items: app.paused },
    { key: 'finished' as const, label: 'Finished', items: app.finished },
  ]);
  const items = $derived(tabs.find((t) => t.key === tab)?.items ?? []);

  function status(d: DownloadView) {
    if (d.state === 'queued') return 'Starts when a slot frees up';
    if (d.state === 'paused') return `Paused at ${pct(d).toFixed(0)}%`;
    return finishedText(d.finishedAt);
  }
</script>

<div class="shelf">
  <div class="tabs" role="tablist" aria-label="Other downloads">
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
              <button class="key sm sq" type="button" aria-label="Open" title="Open" onclick={() => app.run(() => api.openFile(d.id))}><Icon name="arrow-square-out" /></button>
              <button class="key sm sq" type="button" aria-label="Show in folder" title="Show in folder" onclick={() => app.run(() => api.revealFile(d.id))}><Icon name="folder-open" /></button>
              <button class="key sm sq" type="button" aria-label="Check the file" title="Check the file (SHA-256)" onclick={() => (app.verify = d)}><Icon name="check" /></button>
            {:else}
              <button class="key sm sq" type="button" aria-label="Start" title="Start" onclick={() => app.run(() => api.resume(d.id))}><Icon name="play" /></button>
            {/if}
            <button class="key sm sq" type="button" aria-label="Remove from list" title="Remove from list" onclick={() => app.run(() => api.remove(d.id))}><Icon name="x" /></button>
          </span>
        </div>
      {:else}
        <p class="empty">Nothing {tabs.find((t) => t.key === tab)?.label.toLowerCase()} right now.</p>
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
    padding-right: 10px;
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
    text-align: right;
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
