<script lang="ts">
  import { fly } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { ms } from './motion';
  import { app } from './store.svelte';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast" animate:flip={{ duration: ms(200) }} in:fly={{ y: 12, duration: ms(220) }} out:fly={{ y: 8, duration: ms(160) }}>{t.text}</div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    left: 50%;
    bottom: 20px;
    z-index: 50;
    transform: translateX(-50%);
    display: grid;
    gap: 6px;
    justify-items: center;
    pointer-events: none;
  }
  .toast {
    max-width: min(520px, 90vw);
    padding: 9px 14px;
    border-radius: 8px;
    background: var(--ink);
    color: var(--panel);
    font-size: 12.5px;
    box-shadow: 0 12px 30px -10px var(--shadow);
    overflow-wrap: anywhere;
  }
</style>
