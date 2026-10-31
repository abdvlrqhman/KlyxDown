<script lang="ts">
  import { msg, t } from './i18n.svelte';
  import { onMount, tick } from 'svelte';
  import { fade } from 'svelte/transition';
  import { listen } from '@tauri-apps/api/event';
  import { open } from '@tauri-apps/plugin-dialog';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import SpacieMark from './SpacieMark.svelte';
  import { api } from './api';
  import { animate, gsap, ms } from './motion';
  import { app } from './store.svelte';
  import { THEMES, resolveTheme } from './themes';

  // First-run guide: five short steps, each sliding in. Skippable at any point.
  const STEPS = [msg('Welcome'), msg('Look'), msg('Browser'), msg('Startup'), msg('Done')] as const;
  let step = $state(0);
  let body: HTMLDivElement;
  let demo = $state<HTMLDivElement>();
  let extension = $state(false);
  const s = $derived(app.settings);

  onMount(() => {
    api.extensionSeen().then((seen) => (extension ||= seen)).catch(() => {});
    const off = listen('extension-seen', () => (extension = true));
    if (animate()) gsap.from(body.parentElement, { y: 18, autoAlpha: 0, duration: 0.45, ease: 'power3.out' });
    return () => {
      off.then((f) => f());
    };
  });

  // Step 1 visual: one file, four connections filling their parts at once.
  $effect(() => {
    if (step !== 0 || !demo || !animate()) return;
    const cells = [...demo.querySelectorAll('i')];
    const lanes = [0, 1, 2, 3].map((l) => cells.slice(l * 8, l * 8 + 8));
    const css = getComputedStyle(document.documentElement);
    const off = css.getPropertyValue('--led-off').trim();
    const on = css.getPropertyValue('--accent').trim();
    const tl = gsap.timeline({ repeat: -1, repeatDelay: 0.6 });
    tl.set(cells, { backgroundColor: off });
    lanes.forEach((lane, l) => tl.to(lane, { backgroundColor: on, duration: 0.06, stagger: 0.12 + l * 0.05 }, 0.2));
    tl.to({}, { duration: 0.8 });
    return () => {
      tl.kill();
      gsap.set(cells, { clearProps: 'backgroundColor' });
    };
  });

  async function go(next: number) {
    if (next < 0 || next >= STEPS.length) return;
    const dir = next > step ? 1 : -1;
    if (animate()) await gsap.to(body, { x: -24 * dir, autoAlpha: 0, duration: 0.18, ease: 'power2.in' });
    step = next;
    await tick();
    if (animate()) {
      gsap.fromTo(body, { x: 24 * dir, autoAlpha: 0 }, { x: 0, autoAlpha: 1, duration: 0.3, ease: 'power3.out' });
      gsap.from(body.querySelectorAll('.stagger > *'), { y: 8, autoAlpha: 0, stagger: 0.05, duration: 0.3, delay: 0.08 });
    }
  }

  function finish() {
    app.updateSettings({ onboarded: true });
    app.onboarding = false;
  }

  async function pickFolder() {
    const picked = await open({ directory: true, defaultPath: s?.downloadDir || undefined, title: t('Save downloads to') });
    if (typeof picked === 'string') app.updateSettings({ downloadDir: picked });
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowRight') go(step + 1);
    if (e.key === 'ArrowLeft') go(step - 1);
  }
</script>

<svelte:window {onkeydown} />

{#snippet toggle(label: string, on: boolean, change: (v: boolean) => void)}
  <button class="line" type="button" role="switch" aria-checked={on} onclick={() => change(!on)}>
    <span>{label}</span><span class="switch" aria-hidden="true"></span>
  </button>
{/snippet}

<div class="scrim" transition:fade={{ duration: ms(200) }} role="presentation"></div>
<div class="wrap"><div class="guide panel" role="dialog" aria-modal="true" aria-label={t('Welcome to KlyxDown')}>
  <div class="dots" aria-label={t('Step {n} of {total}', { n: step + 1, total: STEPS.length })}>
    {#each STEPS as name, i (name)}
      <button class="dot" class:on={i <= step} type="button" aria-label={t('Go to step {n}: {name}', { n: i + 1, name: t(name) })} onclick={() => go(i)}></button>
    {/each}
  </div>

  <div class="body" bind:this={body}>
    {#if step === 0}
      <div class="stagger">
        <Logo size={56} />
        <h2>{t('Welcome to KlyxDown')}</h2>
        <p>{t('Files download in several parts at the same time, so big downloads finish sooner. Each lane below is one connection filling its part of the file.')}</p>
        <div class="demo" bind:this={demo} aria-hidden="true">
          {#each Array(32) as _, i (i)}<i></i>{/each}
        </div>
      </div>
    {:else if step === 1}
      <div class="stagger">
        <h2>{t('Pick a look')}</h2>
        <p>{t('You can change it any time in Settings.')}</p>
        <div class="themes">
          {#each THEMES as th (th.name)}
            <button class="theme" type="button" data-theme={resolveTheme(th.name)} aria-pressed={s?.theme === th.name} onclick={() => app.updateSettings({ theme: th.name })}>
              <Logo size={26} />
              <span>{t(th.label)}</span>
            </button>
          {/each}
        </div>
      </div>
    {:else if step === 2}
      <div class="stagger">
        <h2>{t('Catch downloads from your browser')}</h2>
        <p>{t('Install the KlyxDown extension in Chrome, Edge, Brave or Firefox. Downloads then come here with their cookies, so sites you are signed in to work too.')}</p>
        <div class="status" class:on={extension}>
          <span class="led" aria-hidden="true"></span>
          <span>{extension ? t('Extension connected') : t('Waiting for the extension. Open its popup once to connect.')}</span>
        </div>
        <button class="key sm" type="button" onclick={() => app.run(api.openProjectPage)}><Icon name="arrow-square-out" />{t('Get it from spacie.net/klyxdown')}</button>
      </div>
    {:else if step === 3}
      <div class="stagger">
        <h2>{t('Set it and forget it')}</h2>
        <p>{t('KlyxDown can wait in the tray and pick up where it left off.')}</p>
        {#if s}
          {@render toggle(t('Run when the computer starts'), s.runOnStartup, (v) => app.updateSettings({ runOnStartup: v }))}
          {@render toggle(t('Minimize to the tray'), s.minimizeToTray, (v) => app.updateSettings({ minimizeToTray: v }))}
          {@render toggle(t('Keep running when the window is closed'), s.closeToTray, (v) => app.updateSettings({ closeToTray: v }))}
          <div class="line folder">
            <span class="path" title={s.downloadDir}>Save to {s.downloadDir}</span>
            <button class="key sm" type="button" onclick={pickFolder}><Icon name="folder-open" />{t('Change')}</button>
          </div>
        {/if}
      </div>
    {:else}
      <div class="stagger">
        <Logo size={56} active />
        <h2>{t('You\'re all set')}</h2>
        <ul class="tips">
          <li>{t('Paste a link anywhere in the window to add it.')}</li>
          <li><kbd>Ctrl</kbd> <kbd>N</kbd> {t('opens the add panel.')}</li>
          <li>{t('Drag links from your browser onto the window.')}</li>
          <li>{t('Hold')} <kbd>Alt</kbd> {t('while clicking a link to let the browser keep it.')}</li>
        </ul>
        <p class="by"><SpacieMark size={16} /><span>{t('KlyxDown is a Spacie project.')}</span></p>
      </div>
    {/if}
  </div>

  <footer>
    {#if step < STEPS.length - 1}
      <button class="key sm skip" type="button" onclick={finish}>{t('Skip the guide')}</button>
    {/if}
    <span class="grow"></span>
    {#if step > 0}<button class="key sm" type="button" onclick={() => go(step - 1)}>{t('Back')}</button>{/if}
    {#if step < STEPS.length - 1}
      <button class="key sm go" type="button" onclick={() => go(step + 1)}>{t('Next')}</button>
    {:else}
      <button class="key sm go" type="button" onclick={finish}>{t('Start downloading')}</button>
    {/if}
  </footer>
</div></div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: color-mix(in srgb, var(--recess) 70%, transparent);
  }
  .wrap {
    position: fixed;
    inset: 0;
    z-index: 51;
    display: grid;
    place-items: center;
    pointer-events: none;
  }
  .guide {
    pointer-events: auto;
    width: min(540px, calc(100% - 32px));
    padding: 22px;
    display: grid;
    gap: 18px;
    overflow: hidden;
    box-shadow: inset 0 1px 0 var(--hi), 0 40px 80px -24px var(--shadow), 0 0 0 1px var(--lo);
  }
  .dots {
    display: flex;
    gap: 6px;
  }
  .dot {
    width: 22px;
    height: 6px;
    padding: 0;
    border: 0;
    border-radius: 2px;
    background: var(--led-off);
    transition: background-color 0.25s var(--ease);
  }
  .dot.on {
    background: var(--accent);
  }
  .body {
    min-height: 260px;
  }
  .stagger {
    display: grid;
    gap: 12px;
    justify-items: start;
  }
  h2 {
    margin: 0;
    font-size: 20px;
    font-stretch: 105%;
    letter-spacing: -0.01em;
  }
  p {
    margin: 0;
    color: var(--ink-2);
    max-width: 52ch;
  }
  .demo {
    width: 100%;
    display: grid;
    grid-template-columns: repeat(32, 1fr);
    gap: 2px;
    height: 16px;
    padding: 4px;
    border-radius: 5px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
  }
  .demo :global(i) {
    border-radius: 1px;
    background: var(--led-off);
  }
  .demo :global(i.on) {
    background: var(--accent);
  }
  .themes {
    width: 100%;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }
  .theme {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px;
    border: 0;
    border-radius: 8px;
    background: var(--chassis);
    color: var(--ink);
    font-weight: 600;
    box-shadow: inset 0 1px 0 var(--hi), 0 0 0 1px var(--lo);
  }
  .theme[aria-pressed='true'] {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 6px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
    font-size: 12.5px;
    color: var(--ink-2);
  }
  .status.on {
    color: var(--ink);
  }
  .led {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--led-dim);
    animation: wait 1.2s ease-in-out infinite;
  }
  .status.on .led {
    background: var(--accent);
    animation: none;
  }
  @keyframes wait {
    50% {
      opacity: 0.3;
    }
  }
  .line {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 0;
    border: 0;
    background: transparent;
    text-align: start;
    font-weight: 600;
  }
  .switch {
    position: relative;
    flex: none;
    width: 38px;
    height: 22px;
    border-radius: 11px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
    transition: background-color 0.2s var(--ease);
  }
  .switch::after {
    content: '';
    position: absolute;
    top: 3px;
    inset-inline-start: 3px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--key);
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 2px var(--lo);
    transition: transform 0.2s var(--ease);
  }
  [aria-checked='true'] .switch {
    background: var(--accent);
  }
  [aria-checked='true'] .switch::after {
    transform: translateX(16px);
  }
  :global([dir='rtl']) [aria-checked='true'] .switch::after {
    transform: translateX(-16px);
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
    color: var(--ink-2);
  }
  .tips {
    margin: 0;
    padding-inline-start: 18px;
    display: grid;
    gap: 6px;
    color: var(--ink-2);
  }
  .by {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  kbd {
    font: 600 11px var(--font);
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--key);
    color: var(--ink);
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grow {
    flex: 1;
  }
  .skip {
    background: transparent;
    box-shadow: none;
    color: var(--ink-2);
  }
</style>
