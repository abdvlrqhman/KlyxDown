<script lang="ts">
  import { t } from './i18n.svelte';
  import { slide } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import { api, errorText } from './api';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { UpdateInfo } from './types';

  // A new version from GitHub Releases: install it now or later. While it downloads,
  // an LED strip fills; then the app restarts into the new version.
  let { update }: { update: UpdateInfo } = $props();

  const CELLS = 40;
  const lit = $derived(
    app.updating?.total ? Math.round((app.updating.received / app.updating.total) * CELLS) : 0,
  );
  const busy = $derived(app.updating !== null);

  async function install() {
    app.updating = { received: 0, total: null };
    try {
      await api.installUpdate();
    } catch (e) {
      app.updating = null;
      app.toast(errorText(e));
    }
  }
</script>

<div class="bar" role="status" transition:slide={{ duration: ms(240) }}>
  <Icon name="tray-arrow-down" />
  {#if busy}
    <span>Downloading KlyxDown {update.version}…</span>
    <div class="leds" aria-hidden="true">
      {#each Array(CELLS) as _, i (i)}<i class:on={i < lit}></i>{/each}
    </div>
  {:else}
    <span><b>KlyxDown {update.version}</b> is ready. You have {update.current}. Downloads pick up where they left off after the restart.</span>
    <button class="key sm" type="button" onclick={() => (app.update = null)}>{t('Later')}</button>
    <button class="key sm go" type="button" onclick={install}>{t('Update and restart')}</button>
  {/if}
</div>

<style>
  .bar {
    grid-area: banner;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px 8px 14px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--accent) 14%, var(--panel));
    font-size: 12.5px;
  }
  .bar :global(.ico) {
    color: var(--accent);
    flex: none;
  }
  span {
    flex: 1;
    min-width: 0;
  }
  .leds {
    flex: 0 1 260px;
    display: grid;
    grid-template-columns: repeat(40, 1fr);
    gap: 1px;
    height: 10px;
    padding: 2px;
    border-radius: 3px;
    background: var(--recess);
  }
  .leds i {
    border-radius: 1px;
    background: var(--led-off);
    transition: background-color 0.2s;
  }
  .leds i.on {
    background: var(--accent);
  }
</style>
