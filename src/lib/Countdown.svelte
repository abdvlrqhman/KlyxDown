<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import { api, errorText } from './api';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import type { FinishAction } from './types';

  // Everything finished and an action was chosen: give the user 30 seconds to stop it.
  // One LED goes out per second.
  let { action }: { action: FinishAction } = $props();

  const SECONDS = 30;
  let left = $state(SECONDS);
  const VERB: Record<FinishAction, string> = {
    nothing: '',
    quit: 'KlyxDown quits',
    sleep: 'The computer goes to sleep',
    shutdown: 'The computer shuts down',
  };

  async function runNow() {
    app.countdown = null;
    try {
      await api.runFinishAction(action);
    } catch (e) {
      app.toast(errorText(e));
    }
  }

  function cancel() {
    app.countdown = null;
    app.toast('Cancelled. Nothing will happen.');
  }

  onMount(() => {
    const timer = setInterval(() => {
      left -= 1;
      if (left <= 0) {
        clearInterval(timer);
        runNow();
      }
    }, 1000);
    return () => clearInterval(timer);
  });
</script>

<div class="scrim" transition:fade={{ duration: ms(160) }} role="presentation"></div>
<div class="wrap">
  <div class="box panel" role="alertdialog" aria-modal="true" aria-labelledby="cd-title" aria-describedby="cd-text" transition:fly={{ y: 12, duration: ms(260) }}>
    <h2 id="cd-title">Everything finished</h2>
    <p id="cd-text">{VERB[action]} in <b class="num">{left}</b> seconds.</p>
    <div class="leds" aria-hidden="true">
      {#each Array(SECONDS) as _, i (i)}<i class:on={i < left}></i>{/each}
    </div>
    <footer>
      <button class="key sm" type="button" onclick={runNow}>Do it now</button>
      <button class="key sm go" type="button" onclick={cancel}><Icon name="x" />Cancel</button>
    </footer>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 55;
    background: color-mix(in srgb, var(--recess) 70%, transparent);
  }
  .wrap {
    position: fixed;
    inset: 0;
    z-index: 56;
    display: grid;
    place-items: center;
    pointer-events: none;
  }
  .box {
    pointer-events: auto;
    width: min(420px, calc(100% - 32px));
    padding: 20px;
    display: grid;
    gap: 14px;
    box-shadow: inset 0 1px 0 var(--hi), 0 40px 80px -24px var(--shadow), 0 0 0 1px var(--lo);
  }
  h2 {
    margin: 0;
    font-size: 18px;
  }
  p {
    margin: 0;
    color: var(--ink-2);
  }
  p b {
    color: var(--ink);
  }
  .leds {
    display: grid;
    grid-template-columns: repeat(30, 1fr);
    gap: 2px;
    height: 12px;
    padding: 3px;
    border-radius: 4px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
  }
  .leds i {
    border-radius: 1px;
    background: var(--led-off);
    transition: background-color 0.3s;
  }
  .leds i.on {
    background: var(--warn);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
