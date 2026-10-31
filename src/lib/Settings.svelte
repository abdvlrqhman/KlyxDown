<script lang="ts">
  import { LANGUAGES, msg, t } from './i18n.svelte';
  import { bytes } from './format';
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
  import type { Settings, ToolStatus } from './types';

  const s = $derived(app.settings);
  const set = (change: Partial<Settings>) => app.updateSettings(change);
  const close = () => (app.settingsOpen = false);
  const PROXY_MODES = [
    { value: 'system' as const, label: msg('Use system settings') },
    { value: 'none' as const, label: msg('No proxy') },
    { value: 'custom' as const, label: msg('Custom proxy') },
  ];
  let proxyUrl = $state(app.settings?.proxyUrl ?? '');
  const languages = $derived([{ value: 'system', label: t('Follow system') }, ...LANGUAGES.map((l) => ({ value: l.code, label: l.name }))]);
  // 0 is "no quota".
  const QUOTA_MB = [0, 100, 250, 500, 1000, 2000, 5000, 10000, 20000];
  const quotas = $derived(QUOTA_MB.map((mb) => ({ value: mb, label: mb ? bytes(mb * 1e6) : t('No quota') })));
  const RATIOS = $derived([
    { value: 0.5, label: t('Until I have shared half of it') },
    { value: 1, label: t('Until I have shared as much as I got') },
    { value: 2, label: t('Until I have shared it twice') },
    { value: 0, label: t('Until I stop it') },
  ]);
  let scanner = $state(app.settings?.scanner ?? '');
  let site = $state({ host: '', user: '', password: '' });
  let siteBusy = $state(false);
  async function addSite(e: SubmitEvent) {
    e.preventDefault();
    siteBusy = true;
    try {
      app.settings = await api.saveSite(site.host, site.user, site.password);
      site = { host: '', user: '', password: '' };
      app.toast(t('Sign-in saved in the system keychain.'));
    } catch (err) {
      app.toast(errorText(err));
    } finally {
      siteBusy = false;
    }
  }
  let tools = $state<ToolStatus | null>(null);
  let toolsBusy = $state(false);
  api.toolsStatus().then((s) => (tools = s)).catch(() => {});
  async function getTools() {
    toolsBusy = true;
    try {
      tools = await api.installTools();
      app.toast(t('yt-dlp and ffmpeg are ready.'));
    } catch (e) {
      app.toast(errorText(e));
    } finally {
      toolsBusy = false;
    }
  }
  const periods = $derived([1, 2, 3, 6, 12, 24].map((n) => ({ value: n, label: t('every {n} hours', { n }) })));
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
      } else app.toast(t('You have the latest version ({version}).', { version }));
    } catch (e) {
      app.toast(errorText(e));
    } finally {
      checking = false;
    }
  }

  async function importList() {
    const path = await open({ title: t('Import links'), filters: [{ name: t('Link lists'), extensions: ['ef2', 'txt', 'lst', 'meta4', 'metalink'] }] });
    if (typeof path !== 'string') return;
    try {
      if (/\.(meta4|metalink)$/i.test(path)) {
        const files = await api.readMetalink(path);
        app.settingsOpen = false;
        app.links = { urls: files.map((f) => f.urls[0]!), files, title: path.split(/[\\/]/).pop() ?? null, headers: [], browser: t('Metalink') };
        return;
      }
      const urls = await api.importLinks(path);
      if (!urls.length) return app.toast(t('No links found in that file.'));
      app.settingsOpen = false;
      app.links = { urls, title: path.split(/[\/]/).pop() ?? null, headers: [], browser: t('Imported list') };
    } catch (e) {
      app.toast(errorText(e));
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true, defaultPath: s?.downloadDir || undefined, title: t('Save downloads to') });
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
      <button class="key sm sq" type="button" aria-label={t('Less')} disabled={value <= min} onclick={() => change(value - 1)}><Icon name="minus" size={14} /></button>
      <output class="num">{value}</output>
      <button class="key sm sq" type="button" aria-label={t('More')} disabled={value >= max} onclick={() => change(value + 1)}><Icon name="plus" size={14} /></button>
    </div>
  </div>
{/snippet}

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="sheet panel" role="dialog" aria-modal="true" aria-labelledby="settings-title" transition:fly={{ x: document.dir === 'rtl' ? -40 : 40, duration: ms(280) }}>
  <header>
    <h2 id="settings-title">{t('Settings')}</h2>
    <button class="key sm sq" type="button" aria-label={t('Close settings')} onclick={close}><Icon name="x" /></button>
  </header>

  {#if s}
    <ScrollArea class="settings-scroll"><div class="scroll">
      <section>
        <h3>{t('Theme')}</h3>
        <div class="themes">
          {#each THEMES as th (th.name)}
            <button class="theme" type="button" data-theme={resolveTheme(th.name)} aria-pressed={s.theme === th.name} onclick={() => set({ theme: th.name })}>
              <span class="preview">
                <Logo size={30} />
                <span class="mini-leds" aria-hidden="true">{#each Array(10) as _, i (i)}<i class:on={i < 6}></i>{/each}</span>
              </span>
              <span class="theme-name">{t(th.label)}</span>
              <span class="theme-note">{t(th.note)}</span>
            </button>
          {/each}
        </div>
      </section>

      <section>
        <div class="line">
          <span class="line-label">{t('Language')}</span>
          <div class="proxy-select"><Select id="language" label={t('Language')} value={s.language} options={languages} onchange={(v) => set({ language: v })} /></div>
        </div>
        {@render toggle(t('Show the intro on launch'), t('A short animation when the window opens.'), s.showIntro, (v) => set({ showIntro: v }))}
      </section>

      <section>
        <h3>{t('Startup')}</h3>
        {@render toggle(t('Run on startup'), t('Start KlyxDown when you sign in to your computer.'), s.runOnStartup, (v) => set({ runOnStartup: v }))}
        {@render toggle(t('Start in the tray'), t('When it starts with your computer, stay out of the way.'), s.startHidden, (v) => set({ startHidden: v }), !s.runOnStartup)}
      </section>

      <section>
        <h3>{t('Window')}</h3>
        {@render toggle(t('Minimize to tray'), t('The minimize button hides the window. Click the tray icon to bring it back.'), s.minimizeToTray, (v) => set({ minimizeToTray: v }))}
        {@render toggle(t('Keep running when closed'), t('The close button hides to the tray so downloads continue.'), s.closeToTray, (v) => set({ closeToTray: v }))}
      </section>

      <section>
        <h3>{t('Browser')}</h3>
        {@render toggle(t('Ask before downloading'), t('Show downloads caught from the browser before they start. Off starts them right away.'), s.askOnCapture, (v) => set({ askOnCapture: v }))}
        {@render toggle(t('Watch the clipboard for links'), t('Copy a link to a file anywhere and KlyxDown offers to download it.'), s.watchClipboard, (v) => set({ watchClipboard: v }))}
        <p class="note">{#each t('Install the KlyxDown extension in Chrome, Edge, Brave or Firefox to catch downloads. Hold {key} while clicking a link to let the browser keep it.').split('{key}') as part, i (i)}{#if i}<kbd>Alt</kbd>{/if}{part}{/each}</p>
      </section>

      <section>
        <h3>{t('Downloads')}</h3>
        <div class="line folder">
          <div>
            <span class="line-label">{t('Save to')}</span>
            <span class="line-hint path" title={s.downloadDir}>{s.downloadDir}</span>
          </div>
          <button class="key sm" type="button" onclick={pickFolder}><Icon name="folder-open" />{t('Change')}</button>
        </div>
        <div class="line">
          <div>
            <span class="line-label">{t('Import links')}</span>
            <span class="line-hint">{t('From an IDM export (.ef2), a Metalink (.meta4) or a text file with one link per line.')}</span>
          </div>
          <button class="key sm" type="button" onclick={importList}><Icon name="tray-arrow-down" />{t('Import')}</button>
        </div>
        {@render toggle(t('Sort into folders by type'), t('Video, Programs, Archives and so on, inside the download folder.'), s.sortByType, (v) => set({ sortByType: v }))}
        {@render toggle(t('Keep the computer awake'), t('Hold off sleep while something is downloading.'), s.keepAwake, (v) => set({ keepAwake: v }))}
        {@render stepper(t('Downloads at the same time'), s.maxActive, 1, 16, (v) => set({ maxActive: v }))}
        {@render stepper(t('Connections per download'), s.connections, 1, 32, (v) => set({ connections: v }))}
      </section>

      <section>
        <h3>{t('Network')}</h3>
        {@render toggle(t('Pause on metered connections'), t('Hold downloads on phone hotspots and capped plans. The banner lets you download anyway.'), s.pauseOnMetered, (v) => set({ pauseOnMetered: v }))}
        <div class="line">
          <div>
            <span class="line-label">{t('Download quota')}</span>
            <span class="line-hint">{t('Stop after this much, until the period is over.')}</span>
          </div>
          <div class="quota">
            <Select id="quota-mb" label={t('Download quota')} value={s.quotaMb ?? 0} options={quotas} onchange={(v) => set({ quotaMb: v || null })} />
            {#if s.quotaMb}<Select id="quota-hours" label={t('Quota period')} value={s.quotaHours} options={periods} onchange={(v) => set({ quotaHours: v })} />{/if}
          </div>
        </div>
        <div class="line">
          <span class="line-label">{t('Proxy')}</span>
          <div class="proxy-select"><Select id="proxy-mode" label={t('Proxy')} value={s.proxyMode} options={PROXY_MODES.map((o) => ({ ...o, label: t(o.label) }))} onchange={(v) => set({ proxyMode: v })} /></div>
        </div>
        {#if s.proxyMode === 'custom'}
          <form class="proxy-url" onsubmit={(e) => (e.preventDefault(), set({ proxyUrl }))}>
            <input class="slot" bind:value={proxyUrl} type="text" placeholder={t('http://host:8080 or socks5://host:1080')} aria-label={t('Proxy address')} spellcheck="false" />
            <button class="key sm" type="submit">{t('Save')}</button>
          </form>
          <p class="note">{t('Add')} <b>{t('user:password@')}</b> {t('before the host if the proxy needs a sign-in.')}</p>
        {/if}
      </section>

      <section>
        <h3>{t('Safety')}</h3>
        {@render toggle(t('Check finished files with the antivirus'), t('Windows asks your installed antivirus. On macOS and Linux, ClamAV is used when installed, or the command below.'), s.scanAfter, (v) => set({ scanAfter: v }))}
        {#if s.scanAfter}
          <form class="proxy-url" onsubmit={(e) => (e.preventDefault(), set({ scanner }))}>
            <input class="slot" bind:value={scanner} type="text" placeholder={t('Your own scanner, for example: clamscan --no-summary {file}')} aria-label={t('Scanner command')} spellcheck="false" />
            <button class="key sm" type="submit">{t('Save')}</button>
          </form>
        {/if}
      </section>

      <section>
        <h3>{t('Sign-ins')}</h3>
        <p class="note">{t('For sites and FTP servers that ask for a user name and password. Passwords are kept in your system keychain, not by KlyxDown.')}</p>
        {#each s.sites as x (x.host)}
          <div class="line">
            <div>
              <span class="line-label">{x.host}</span>
              <span class="line-hint">{x.user}</span>
            </div>
            <button class="key sm sq" type="button" aria-label={t('Remove sign-in')} title={t('Remove sign-in')} onclick={async () => { try { app.settings = await api.removeSite(x.host); } catch (e) { app.toast(errorText(e)); } }}><Icon name="trash" /></button>
          </div>
        {/each}
        <form class="site" onsubmit={addSite}>
          <input class="slot" bind:value={site.host} type="text" placeholder={t('Site, like files.example.com')} aria-label={t('Site')} spellcheck="false" autocomplete="off" />
          <input class="slot" bind:value={site.user} type="text" placeholder={t('User name')} aria-label={t('User name')} spellcheck="false" autocomplete="off" />
          <input class="slot" bind:value={site.password} type="password" placeholder={t('Password')} aria-label={t('Password')} autocomplete="new-password" />
          <button class="key sm" type="submit" disabled={siteBusy || !site.host || !site.user || !site.password}><Icon name="key" />{t('Save')}</button>
        </form>
      </section>

      <section>
        <h3>{t('Torrents')}</h3>
        {@render toggle(t('Keep sharing finished torrents'), t('Torrents work because people share what they downloaded. Sharing stops on its own at the goal below.'), s.seed, (v) => set({ seed: v }))}
        {#if s.seed}
          <div class="line">
            <span class="line-label">{t('Share')}</span>
            <div class="proxy-select wide"><Select id="seed-ratio" label={t('Share')} value={s.seedRatio} options={RATIOS} onchange={(v) => set({ seedRatio: v })} /></div>
          </div>
        {/if}
        {@render toggle(t('Open magnet links with KlyxDown'), t('Magnet links clicked in the browser and other apps come here.'), s.handleMagnets, (v) => set({ handleMagnets: v }))}
      </section>

      <section>
        <h3>{t('Video from web pages')}</h3>
        <div class="line">
          <div>
            <span class="line-label">yt-dlp {tools?.ytdlp ?? ''}</span>
            <span class="line-hint">
              {tools?.ytdlp ? t('Installed. It updates itself once a day, as sites change often.') : t('Not installed yet. KlyxDown gets it the first time you download a video.')}
              {tools && !tools.ffmpeg ? ' ' + (tools.ffmpegDownloadable ? t('ffmpeg comes with it.') : t('Install ffmpeg from your package manager to merge picture and sound.')) : ''}
            </span>
          </div>
          {#if !tools?.ytdlp || (!tools.ffmpeg && tools.ffmpegDownloadable)}
            <button class="key sm" type="button" disabled={toolsBusy} onclick={getTools}><Icon name="download-simple" />{toolsBusy ? t('Downloading…') : t('Get them now')}</button>
          {/if}
        </div>
      </section>

      <section class="about">
        <Logo size={40} />
        <div>
          <span class="line-label">KlyxDown {version}</span>
          <span class="line-hint">{t('Open-source download manager. No ads, no tracking.')}{portable ? ' ' + t('Running in portable mode.') : ''}</span>
        </div>
        <button class="key sm" type="button" onclick={() => api.quit()}><Icon name="power" />{t('Quit')}</button>
      </section>
      <section>
        <h3>{t('Updates')}</h3>
        {@render toggle(t('Check for updates automatically'), t('Looks for a new version on GitHub a few seconds after launch.'), s.autoUpdate, (v) => set({ autoUpdate: v }))}
        <div class="line">
          <span class="line-hint">{t('Updates come from github.com/abdvlrqhman/KlyxDown and are signature-checked before they install.')}</span>
          <button class="key sm" type="button" disabled={checking} onclick={checkNow}><Icon name="arrow-clockwise" />{checking ? t('Checking…') : t('Check now')}</button>
        </div>
      </section>

      <section class="spacie">
        <SpacieMark size={22} />
        <span class="line-hint">{t('A Spacie project')}</span>
        <button class="key sm" type="button" onclick={() => app.run(api.openProjectPage)}><Icon name="arrow-square-out" />{t('spacie.net/klyxdown')}</button>
        <button class="key sm" type="button" onclick={() => ((app.settingsOpen = false), (app.onboarding = true))}>{t('Welcome guide')}</button>
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
    inset-inline-end: 8px;
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
    text-align: start;
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
    inset-inline-start: 3px;
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
  :global([dir='rtl']) .switch[aria-checked='true']::after {
    transform: translateX(-16px);
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
  .site {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr) minmax(0, 1fr) auto;
    gap: 6px;
  }
  .proxy-select.wide {
    width: 230px;
  }
  .quota {
    display: grid;
    gap: 6px;
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
