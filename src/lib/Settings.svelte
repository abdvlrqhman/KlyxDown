<script lang="ts">
  import { fade, fly } from 'svelte/transition';
  import { getVersion } from '@tauri-apps/api/app';
  import { open } from '@tauri-apps/plugin-dialog';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import Select from './Select.svelte';
  import SpacieMark from './SpacieMark.svelte';
  import ScrollArea from './ScrollArea.svelte';
  import { api, errorText } from './api';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import { THEMES, resolveTheme } from './themes';
  import type { Settings } from './types';

  const s = $derived(app.settings);
  const set = (change: Partial<Settings>) => app.updateSettings(change);
  const close = () => (app.settingsOpen = false);
  const PROXY_MODES = [
    { value: 'system' as const, label: 'Use system settings' },
    { value: 'none' as const, label: 'No proxy' },
    { value: 'custom' as const, label: 'Custom proxy' },
  ];
  let proxyUrl = $state(app.settings?.proxyUrl ?? '');
  let portable = $state(false);
  let version = $state('');
  let checking = $state(false);
  api.isPortable().then((p) => (portable = p)).catch(() => {});
  getVersion().then((v) => (version = v)).catch(() => {});

  async function checkNow() {
    checking = true;
    try {
      const found = await api.checkUpdate();
      if (found) {
        app.update = found;
        app.settingsOpen = false;
      } else app.toast(`You have the latest version (${version}).`);
    } catch (e) {
      app.toast(errorText(e));
    } finally {
      checking = false;
    }
  }

  async function importList() {
    const path = await open({ title: 'Import links', filters: [{ name: 'Link lists', extensions: ['ef2', 'txt', 'lst'] }] });
    if (typeof path !== 'string') return;
    try {
      const urls = await api.importLinks(path);
      if (!urls.length) return app.toast('No links found in that file.');
      app.settingsOpen = false;
      app.links = { urls, title: path.split(/[\/]/).pop() ?? null, headers: [], browser: 'Imported list' };
    } catch (e) {
      app.toast(errorText(e));
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true, defaultPath: s?.downloadDir || undefined, title: 'Save downloads to' });
    if (typeof picked === 'string') set({ downloadDir: picked });
  }
</script>

{#snippet toggle(label: string, hint: string, on: boolean, change: (v: boolean) => void, disabled = false)}
  <div class="line" class:disabled>
    <div>
      <span class="line-label">{label}</span>
      <span class="line-hint">{hint}</span>
    </div>
    <button class="switch" type="button" role="switch" aria-checked={on} aria-label={label} {disabled} onclick={() => change(!on)}></button>
  </div>
{/snippet}

{#snippet stepper(label: string, value: number, min: number, max: number, change: (v: number) => void)}
  <div class="line">
    <span class="line-label">{label}</span>
    <div class="stepper">
      <button class="key sm sq" type="button" aria-label="Less" disabled={value <= min} onclick={() => change(value - 1)}><Icon name="minus" size={14} /></button>
      <output class="num">{value}</output>
      <button class="key sm sq" type="button" aria-label="More" disabled={value >= max} onclick={() => change(value + 1)}><Icon name="plus" size={14} /></button>
    </div>
  </div>
{/snippet}

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="sheet panel" role="dialog" aria-modal="true" aria-labelledby="settings-title" transition:fly={{ x: 40, duration: ms(280) }}>
  <header>
    <h2 id="settings-title">Settings</h2>
    <button class="key sm sq" type="button" aria-label="Close settings" onclick={close}><Icon name="x" /></button>
  </header>

  {#if s}
    <ScrollArea class="settings-scroll"><div class="scroll">
      <section>
        <h3>Theme</h3>
        <div class="themes">
          {#each THEMES as t (t.name)}
            <button class="theme" type="button" data-theme={resolveTheme(t.name)} aria-pressed={s.theme === t.name} onclick={() => set({ theme: t.name })}>
              <span class="preview">
                <Logo size={30} />
                <span class="mini-leds" aria-hidden="true">{#each Array(10) as _, i (i)}<i class:on={i < 6}></i>{/each}</span>
              </span>
              <span class="theme-name">{t.label}</span>
              <span class="theme-note">{t.note}</span>
            </button>
          {/each}
        </div>
      </section>

      <section>
        {@render toggle('Show the intro on launch', 'A short animation when the window opens.', s.showIntro, (v) => set({ showIntro: v }))}
      </section>

      <section>
        <h3>Startup</h3>
        {@render toggle('Run on startup', 'Start KlyxDown when you sign in to your computer.', s.runOnStartup, (v) => set({ runOnStartup: v }))}
        {@render toggle('Start in the tray', 'When it starts with your computer, stay out of the way.', s.startHidden, (v) => set({ startHidden: v }), !s.runOnStartup)}
      </section>

      <section>
        <h3>Window</h3>
        {@render toggle('Minimize to tray', 'The minimize button hides the window. Click the tray icon to bring it back.', s.minimizeToTray, (v) => set({ minimizeToTray: v }))}
        {@render toggle('Keep running when closed', 'The close button hides to the tray so downloads continue.', s.closeToTray, (v) => set({ closeToTray: v }))}
      </section>

      <section>
        <h3>Browser</h3>
        {@render toggle('Ask before downloading', 'Show downloads caught from the browser before they start. Off starts them right away.', s.askOnCapture, (v) => set({ askOnCapture: v }))}
        {@render toggle('Watch the clipboard for links', 'Copy a link to a file anywhere and KlyxDown offers to download it.', s.watchClipboard, (v) => set({ watchClipboard: v }))}
        <p class="note">Install the KlyxDown extension in Chrome, Edge, Brave or Firefox to catch downloads. Hold <kbd>Alt</kbd> while clicking a link to let the browser keep it.</p>
      </section>

      <section>
        <h3>Downloads</h3>
        <div class="line folder">
          <div>
            <span class="line-label">Save to</span>
            <span class="line-hint path" title={s.downloadDir}>{s.downloadDir}</span>
          </div>
          <button class="key sm" type="button" onclick={pickFolder}><Icon name="folder-open" />Change</button>
        </div>
        <div class="line">
          <div>
            <span class="line-label">Import links</span>
            <span class="line-hint">From an IDM export (.ef2) or a text file with one link per line.</span>
          </div>
          <button class="key sm" type="button" onclick={importList}><Icon name="tray-arrow-down" />Import</button>
        </div>
        {@render toggle('Sort into folders by type', 'Video, Programs, Archives and so on, inside the download folder.', s.sortByType, (v) => set({ sortByType: v }))}
        {@render toggle('Keep the computer awake', 'Hold off sleep while something is downloading.', s.keepAwake, (v) => set({ keepAwake: v }))}
        {@render stepper('Downloads at the same time', s.maxActive, 1, 16, (v) => set({ maxActive: v }))}
        {@render stepper('Connections per download', s.connections, 1, 32, (v) => set({ connections: v }))}
      </section>

      <section>
        <h3>Network</h3>
        <div class="line">
          <span class="line-label">Proxy</span>
          <div class="proxy-select"><Select id="proxy-mode" label="Proxy" value={s.proxyMode} options={PROXY_MODES} onchange={(v) => set({ proxyMode: v })} /></div>
        </div>
        {#if s.proxyMode === 'custom'}
          <form class="proxy-url" onsubmit={(e) => (e.preventDefault(), set({ proxyUrl }))}>
            <input class="slot" bind:value={proxyUrl} type="text" placeholder="http://host:8080 or socks5://host:1080" aria-label="Proxy address" spellcheck="false" />
            <button class="key sm" type="submit">Save</button>
          </form>
          <p class="note">Add <b>user:password@</b> before the host if the proxy needs a sign-in.</p>
        {/if}
      </section>

      <section class="about">
        <Logo size={40} />
        <div>
          <span class="line-label">KlyxDown {version}</span>
          <span class="line-hint">Open-source download manager. No ads, no tracking.{portable ? ' Running in portable mode.' : ''}</span>
        </div>
        <button class="key sm" type="button" onclick={() => api.quit()}><Icon name="power" />Quit</button>
      </section>
      <section>
        <h3>Updates</h3>
        {@render toggle('Check for updates automatically', 'Looks for a new version on GitHub a few seconds after launch.', s.autoUpdate, (v) => set({ autoUpdate: v }))}
        <div class="line">
          <span class="line-hint">Updates come from github.com/abdvlrqhman/KlyxDown and are signature-checked before they install.</span>
          <button class="key sm" type="button" disabled={checking} onclick={checkNow}><Icon name="arrow-clockwise" />{checking ? 'Checking…' : 'Check now'}</button>
        </div>
      </section>

      <section class="spacie">
        <SpacieMark size={22} />
        <span class="line-hint">A Spacie project</span>
        <button class="key sm" type="button" onclick={() => app.run(api.openProjectPage)}><Icon name="arrow-square-out" />spacie.net/klyxdown</button>
        <button class="key sm" type="button" onclick={() => ((app.settingsOpen = false), (app.onboarding = true))}>Welcome guide</button>
      </section>
    </div></ScrollArea>
  {/if}
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: color-mix(in srgb, var(--recess) 55%, transparent);
  }
  .sheet {
    position: fixed;
    z-index: 31;
    top: 8px;
    right: 8px;
    bottom: 8px;
    width: min(440px, calc(100% - 16px));
    display: flex;
    flex-direction: column;
    box-shadow: inset 0 1px 0 var(--hi), -20px 0 60px -20px var(--shadow), 0 0 0 1px var(--lo);
  }
  header {
    display: flex;
    align-items: center;
    padding: 14px 14px 10px 18px;
  }
  header h2 {
    flex: 1;
    margin: 0;
    font-size: 16px;
    font-stretch: 95%;
  }
  .sheet :global(.settings-scroll) {
    flex: 1;
  }
  .scroll {
    padding: 0 18px 18px;
    display: grid;
    gap: 22px;
  }
  section {
    display: grid;
    gap: 12px;
  }
  h3 {
    margin: 0;
    font-size: 11.5px;
    font-weight: 600;
    font-stretch: 85%;
    color: var(--ink-2);
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  .theme {
    display: grid;
    gap: 2px;
    justify-items: start;
    padding: 10px;
    border: 0;
    border-radius: 8px;
    text-align: left;
    background: var(--chassis);
    color: var(--ink);
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo), 0 0 0 1px var(--lo);
    transition: transform 0.1s;
  }
  .theme:active {
    transform: translateY(1px);
  }
  .theme[aria-pressed='true'] {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .preview {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    margin-bottom: 6px;
  }
  .mini-leds {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(10, 1fr);
    gap: 2px;
    height: 10px;
    padding: 2px;
    border-radius: 3px;
    background: var(--recess);
  }
  .mini-leds i {
    border-radius: 1px;
    background: var(--led-off);
  }
  .mini-leds i.on {
    background: var(--accent);
  }
  .theme-name {
    font-weight: 650;
  }
  .theme-note {
    font-size: 11.5px;
    color: var(--ink-2);
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .line > div {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .line.disabled {
    opacity: 0.55;
  }
  .line-label {
    font-weight: 600;
  }
  .line-hint {
    font-size: 12px;
    color: var(--ink-2);
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .switch {
    flex: none;
    position: relative;
    width: 38px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 11px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
    transition: background-color 0.2s var(--ease);
  }
  .switch::after {
    content: '';
    position: absolute;
    top: 3px;
    left: 3px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--key);
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 2px var(--lo);
    transition: transform 0.2s var(--ease);
  }
  .switch[aria-checked='true'] {
    background: var(--accent);
  }
  .switch[aria-checked='true']::after {
    transform: translateX(16px);
  }
  .switch:disabled {
    cursor: default;
  }
  .stepper {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .stepper output {
    min-width: 24px;
    text-align: center;
    font-weight: 650;
  }
  .note {
    margin: 0;
    font-size: 12px;
    color: var(--ink-2);
  }
  kbd {
    font: 600 11px var(--font);
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--key);
    box-shadow: inset 0 1px 0 var(--hi), 0 1px 0 var(--lo);
  }
  .proxy-select {
    width: 190px;
  }
  .proxy-url {
    display: flex;
    gap: 8px;
  }
  .proxy-url .slot {
    flex: 1;
    min-width: 0;
  }
  .spacie {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
  }
  .spacie .line-hint {
    flex: 1;
  }
  .about {
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    padding-top: 14px;
  }
  .about > div {
    display: grid;
    gap: 2px;
  }
</style>
