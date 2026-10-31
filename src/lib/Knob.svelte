<script lang="ts">
  import { msg, t } from './i18n.svelte';
  import { onMount } from 'svelte';
  import { Draggable, animate, gsap } from './motion';
  import type { SpeedMode } from './types';

  let { mode, onchange }: { mode: SpeedMode; onchange: (mode: SpeedMode) => void } = $props();

  const ORDER: SpeedMode[] = ['snail', 'balanced', 'full'];
  const ANGLE: Record<SpeedMode, number> = { snail: -120, balanced: 0, full: 120 };
  const LABEL: Record<SpeedMode, string> = { snail: msg('Snail'), balanced: msg('Balanced'), full: msg('Full') };
  const MARKS = [-120, -90, -60, -30, 0, 30, 60, 90, 120];

  let knob: HTMLDivElement;

  const nearest = (deg: number) =>
    ORDER.reduce((a, b) => (Math.abs(ANGLE[b] - deg) < Math.abs(ANGLE[a] - deg) ? b : a));
  const turnTo = (m: SpeedMode) =>
    gsap.to(knob, { rotation: ANGLE[m], duration: animate() ? 0.55 : 0, ease: 'back.out(2)' });
  const step = (dir: number) => onchange(ORDER[Math.max(0, Math.min(2, ORDER.indexOf(mode) + dir))]!);
  const next = () => onchange(ORDER[(ORDER.indexOf(mode) + 1) % 3]!);

  $effect(() => {
    turnTo(mode);
  });

  onMount(() => {
    gsap.set(knob, { rotation: ANGLE[mode] });
    const [drag] = Draggable.create(knob, {
      type: 'rotation',
      bounds: { minRotation: -120, maxRotation: 120 },
      onClick: next,
      onDragEnd() {
        const m = nearest(this.rotation);
        if (m === mode) turnTo(m);
        else onchange(m);
      },
    });
    return () => drag?.kill();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowRight' || e.key === 'ArrowUp') step(1);
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') step(-1);
    else if (e.key === 'Enter' || e.key === ' ') next();
    else return;
    e.preventDefault();
  }
</script>

<div class="dial">
  <div class="marks" aria-hidden="true">
    {#each MARKS as a (a)}
      <i class:on={a <= ANGLE[mode]} style:transform={`rotate(${a}deg)`}></i>
    {/each}
  </div>
  <div
    class="knob"
    bind:this={knob}
    role="slider"
    tabindex="0"
    aria-label={t('Speed mode')}
    aria-valuemin={0}
    aria-valuemax={2}
    aria-valuenow={ORDER.indexOf(mode)}
    aria-valuetext={t(LABEL[mode])}
    {onkeydown}
  ></div>
  {#each ORDER as m (m)}
    <button class="ml {m}" type="button" aria-pressed={mode === m} onclick={() => onchange(m)}>{t(LABEL[m])}</button>
  {/each}
</div>

<style>
  .dial {
    position: relative;
    width: 172px;
    height: 150px;
    margin: 0 auto;
  }
  .marks {
    position: absolute;
    left: 50%;
    top: 14px;
    width: 120px;
    height: 120px;
    margin-left: -60px;
  }
  .marks i {
    position: absolute;
    left: 50%;
    top: 0;
    width: 2px;
    height: 120px;
    margin-left: -1px;
  }
  .marks i::before {
    content: '';
    position: absolute;
    left: 0;
    top: 0;
    width: 2px;
    height: 7px;
    border-radius: 1px;
    background: var(--ink-3);
    transition: background-color 0.2s, height 0.2s;
  }
  .marks i.on::before {
    background: var(--accent);
    height: 9px;
  }
  .knob {
    position: absolute;
    left: 50%;
    top: 30px;
    width: 88px;
    height: 88px;
    margin-left: -44px;
    border-radius: 50%;
    background: var(--key);
    box-shadow: inset 0 2px 0 var(--hi), 0 2px 0 var(--lo), 0 10px 18px -8px var(--shadow);
    cursor: grab;
    touch-action: none;
  }
  .knob:active {
    cursor: grabbing;
  }
  .knob::after {
    content: '';
    position: absolute;
    left: 50%;
    top: 8px;
    width: 3px;
    height: 20px;
    margin-left: -1.5px;
    border-radius: 2px;
    background: var(--accent);
  }
  .ml {
    position: absolute;
    border: 0;
    background: transparent;
    padding: 2px 4px;
    border-radius: 4px;
    font-size: 11.5px;
    font-weight: 600;
    font-stretch: 85%;
    color: var(--ink-3);
  }
  .ml[aria-pressed='true'] {
    color: var(--ink);
  }
  .ml.snail {
    left: 0;
    bottom: 6px;
  }
  .ml.balanced {
    left: 50%;
    top: -6px;
    transform: translateX(-50%);
  }
  .ml.full {
    right: 2px;
    bottom: 6px;
  }
</style>
