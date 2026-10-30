<script lang="ts">
  import type { DownloadView, SegmentView } from './types';

  // A file map: each cell is one slice of the file, lit when that slice is on disk.
  let {
    d,
    cells = 48,
    segment = null,
    thin = false,
  }: { d: DownloadView; cells?: number; segment?: SegmentView | null; thin?: boolean } = $props();

  const levels = $derived.by(() => {
    const size = d.size ?? 0;
    if (!size) return new Array<number>(cells).fill(d.state === 'done' ? 2 : 0);
    const segs = segment ? [segment] : d.segments;
    const out = new Array<number>(cells);
    for (let k = 0; k < cells; k++) {
      const lo = (size * k) / cells;
      const hi = (size * (k + 1)) / cells;
      let got = 0;
      for (const s of segs) got += Math.max(0, Math.min(hi, s.pos) - Math.max(lo, s.start));
      const f = got / (hi - lo);
      out[k] = f > 0.98 ? 2 : f > 0.02 ? 1 : 0;
    }
    return out;
  });
</script>

<div class="leds" class:thin style:--n={cells} data-state={d.state} aria-hidden="true">
  {#each levels as level, i (i)}
    <i class:h={level === 1} class:on={level === 2}></i>
  {/each}
</div>

<style>
  .leds {
    display: grid;
    grid-template-columns: repeat(var(--n), 1fr);
    gap: 1px;
    height: 12px;
    padding: 3px;
    border-radius: 4px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
  }
  .leds.thin {
    height: 8px;
    padding: 2px;
  }
  i {
    border-radius: 1px;
    background: var(--led-off);
    transition: background-color 0.2s;
  }
  i.h {
    background: var(--led-dim);
  }
  i.on {
    background: var(--accent);
  }
  [data-state='failed'] i.on {
    background: var(--warn);
  }
  [data-state='paused'] i.on,
  [data-state='queued'] i.on {
    background: var(--ink-3);
  }
</style>
