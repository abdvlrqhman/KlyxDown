<script lang="ts">
  import type { Snippet } from 'svelte';

  // Native scrolling (wheel, touch, keyboard, focus) with a custom Tactile thumb.
  // The native bar is hidden; the thumb mirrors the viewport and can be dragged.
  let { children, class: cls = '' }: { children: Snippet; class?: string } = $props();

  let viewport: HTMLDivElement;
  let thumbSize = $state(0);
  let thumbPos = $state(0);
  let scrollable = $state(false);
  let active = $state(false);
  let dragging = $state(false);
  let idle: ReturnType<typeof setTimeout> | undefined;

  const MIN_THUMB = 28;

  function measure() {
    const { scrollHeight, clientHeight, scrollTop } = viewport;
    scrollable = scrollHeight > clientHeight + 1;
    if (!scrollable) return;
    const track = clientHeight - 8;
    thumbSize = Math.max(MIN_THUMB, (clientHeight / scrollHeight) * track);
    thumbPos = (scrollTop / (scrollHeight - clientHeight)) * (track - thumbSize);
  }

  function wake() {
    active = true;
    clearTimeout(idle);
    idle = setTimeout(() => (active = false), 900);
  }

  function onscroll() {
    measure();
    wake();
  }

  $effect(() => {
    // Content or size changes (rows added, window resized) change the thumb.
    const ro = new ResizeObserver(measure);
    ro.observe(viewport);
    const mo = new MutationObserver(measure);
    mo.observe(viewport, { childList: true, subtree: true });
    measure();
    return () => {
      ro.disconnect();
      mo.disconnect();
      clearTimeout(idle);
    };
  });

  function startDrag(e: PointerEvent) {
    e.preventDefault();
    const thumb = e.currentTarget as HTMLElement;
    thumb.setPointerCapture(e.pointerId);
    dragging = true;
    const startY = e.clientY;
    const startTop = viewport.scrollTop;
    const ratio = (viewport.scrollHeight - viewport.clientHeight) / (viewport.clientHeight - 8 - thumbSize);
    const move = (ev: PointerEvent) => (viewport.scrollTop = startTop + (ev.clientY - startY) * ratio);
    const up = () => {
      dragging = false;
      thumb.removeEventListener('pointermove', move);
      thumb.removeEventListener('pointerup', up);
      thumb.removeEventListener('pointercancel', up);
    };
    thumb.addEventListener('pointermove', move);
    thumb.addEventListener('pointerup', up);
    thumb.addEventListener('pointercancel', up);
  }

  // Clicking the track pages toward the click, like a native bar.
  function onTrack(e: PointerEvent) {
    if (e.target !== e.currentTarget) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const dir = e.clientY - rect.top < thumbPos ? -1 : 1;
    viewport.scrollBy({ top: dir * viewport.clientHeight * 0.9, behavior: 'smooth' });
  }
</script>

<div class="area {cls}">
  <div class="viewport" bind:this={viewport} {onscroll}>
    {@render children()}
  </div>
  {#if scrollable}
    <!-- Pointer-only extra: wheel, keyboard and touch scrolling stay native. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="track" class:show={active || dragging} onpointerdown={onTrack} onpointerenter={wake} aria-hidden="true">
      <div class="thumb" class:dragging style:height="{thumbSize}px" style:transform="translateY({thumbPos}px)" onpointerdown={startDrag}></div>
    </div>
  {/if}
</div>

<style>
  .area {
    position: relative;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .viewport {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-width: none;
  }
  .viewport::-webkit-scrollbar {
    display: none;
  }
  .track {
    position: absolute;
    top: 4px;
    inset-inline-end: 2px;
    bottom: 4px;
    width: 8px;
    border-radius: 4px;
    opacity: 0;
    transition: opacity 0.25s var(--ease), background-color 0.2s;
  }
  .area:hover .track,
  .track.show {
    opacity: 1;
  }
  .track:hover {
    background: color-mix(in srgb, var(--recess) 70%, transparent);
  }
  .thumb {
    position: absolute;
    left: 1px;
    right: 1px;
    top: 0;
    border-radius: 3px;
    background: var(--ink-3);
    opacity: 0.55;
    transition: opacity 0.15s, background-color 0.15s;
    touch-action: none;
  }
  .track:hover .thumb,
  .thumb.dragging {
    opacity: 1;
    background: var(--accent);
  }
</style>
