<script lang="ts">
  import { t } from './i18n.svelte';
  import { onMount } from 'svelte';
  import Logo from './Logo.svelte';
  import SpacieMark from './SpacieMark.svelte';
  import { animate, gsap } from './motion';

  // Launch intro: the key cap appears, its LEDs light row by row, the arrow drops,
  // the name rises in. About 1.6 s; any click or key skips it.
  let { ondone }: { ondone: () => void } = $props();

  let root: HTMLDivElement;
  const NAME = 'KlyxDown';
  let finished = false;

  function finish() {
    if (finished) return;
    finished = true;
    ondone();
  }

  onMount(() => {
    if (!animate()) {
      finish();
      return;
    }
    const css = getComputedStyle(document.documentElement);
    const off = css.getPropertyValue('--led-off').trim();
    const accent = css.getPropertyValue('--accent').trim();
    const cells = root.querySelectorAll<SVGRectElement>('.cell');
    const lit = root.querySelectorAll<SVGRectElement>('.cell.on');

    const tl = gsap.timeline({ onComplete: finish });
    tl.from(root.querySelector('.cap'), { scale: 0.6, autoAlpha: 0, transformOrigin: '50% 50%', duration: 0.4, ease: 'back.out(1.7)' })
      .from(cells, { scale: 0, transformOrigin: '50% 50%', duration: 0.22, stagger: { each: 0.014, grid: [5, 5], from: 'start' } }, '-=0.15')
      .fromTo(lit, { fill: off }, { fill: accent, duration: 0.14, stagger: 0.05 }, '-=0.05')
      .to(lit, { y: 4, duration: 0.12, ease: 'power2.in', yoyo: true, repeat: 1 })
      .from(root.querySelectorAll('.name span'), { yPercent: 110, autoAlpha: 0, duration: 0.36, stagger: 0.03, ease: 'power3.out' }, '-=0.25')
      .from(root.querySelector('.by'), { autoAlpha: 0, y: 6, duration: 0.3 }, '-=0.15')
      .to({}, { duration: 0.35 })
      .to(root, { autoAlpha: 0, scale: 1.03, duration: 0.35, ease: 'power2.in' })
      .set([cells, lit], { clearProps: 'all' });

    const skip = () => tl.progress(1);
    addEventListener('keydown', skip, { once: true });
    return () => {
      removeEventListener('keydown', skip);
      tl.kill();
    };
  });
</script>

<div class="intro" bind:this={root} onclick={() => finish()} role="presentation">
  <div class="center">
    <Logo size={112} />
    <h1 class="name" dir="ltr" aria-label={NAME}>
      {#each NAME as ch, i (i)}<span aria-hidden="true">{ch}</span>{/each}
    </h1>
  </div>
  <div class="by"><SpacieMark size={16} /><span>{t('by Spacie')}</span></div>
</div>

<style>
  .intro {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: grid;
    place-items: center;
    background: var(--chassis);
    cursor: default;
  }
  .center {
    display: grid;
    justify-items: center;
    gap: 18px;
  }
  .name {
    display: flex;
    margin: 0;
    overflow: hidden;
    font-size: 34px;
    font-weight: 800;
    font-stretch: 118%;
    letter-spacing: -0.02em;
    line-height: 1.15;
  }
  .name span {
    display: inline-block;
  }
  .by {
    position: absolute;
    bottom: 28px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--ink-2);
  }
</style>
