<script lang="ts">
  import { t } from './i18n.svelte';
  // The KlyxDown mark: a key cap with a 5x5 LED matrix showing a down arrow.
  // Colors come from the active theme, so the mark matches every theme.
  // `active` makes the arrow flow downward while something is downloading.
  let { size = 20, active = false }: { size?: number; active?: boolean } = $props();

  const ARROW = new Set(['2,0', '2,1', '2,2', '2,3', '2,4', '0,2', '4,2', '1,3', '3,3']);
  const cells = Array.from({ length: 25 }, (_, i) => {
    const c = i % 5;
    const r = Math.floor(i / 5);
    return { x: 8 + c * 10, y: 8 + r * 10, r, on: ARROW.has(`${c},${r}`) };
  });
</script>

<svg class:active width={size} height={size} viewBox="0 0 64 64" role="img" aria-label={t('KlyxDown')}>
  <rect class="cap" width="64" height="64" rx="15" />
  <rect class="edge" x="0.5" y="0.5" width="63" height="63" rx="14.5" />
  {#each cells as cell (cell.x * 100 + cell.y)}
    <rect class="cell" class:on={cell.on} style:--row={cell.r} x={cell.x} y={cell.y} width="8" height="8" rx="2" />
  {/each}
</svg>

<style>
  svg {
    flex: none;
    display: block;
  }
  .cap {
    fill: var(--logo-bg);
    transition: fill 0.25s var(--ease);
  }
  .edge {
    fill: none;
    stroke: var(--logo-edge);
  }
  .cell {
    fill: var(--led-off);
    transition: fill 0.25s var(--ease);
  }
  .cell.on {
    fill: var(--accent);
  }
  @media (prefers-reduced-motion: no-preference) {
    .active .cell.on {
      animation: flow 1.2s var(--ease) infinite;
      animation-delay: calc(var(--row) * 0.12s);
    }
  }
  @keyframes flow {
    0%,
    100% {
      opacity: 1;
    }
    40% {
      opacity: 0.35;
    }
  }
</style>
