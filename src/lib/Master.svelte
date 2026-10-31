<script lang="ts">
  import { msg, t } from './i18n.svelte';
  import Icon from './Icon.svelte';
  import Knob from './Knob.svelte';
  import Select from './Select.svelte';
  import { api } from './api';
  import { mbps } from './format';
  import { app } from './store.svelte';
  import type { FinishAction, SpeedMode } from './types';

  const NOTE: Record<SpeedMode, string> = {
    snail: msg('Leaves almost all bandwidth for calls and browsing.'),
    balanced: msg('Keeps browsing smooth while downloads run.'),
    full: msg('Uses everything your connection gives.'),
  };
  // Speed limit steps in MB/s; 0 means no limit.
  const LIMITS = [0, 1, 2, 5, 10, 20, 50, 100];
  const COLUMNS = 18;
  const ROWS = 8;

  const mode = $derived(app.settings?.mode ?? 'balanced');
  const limitMb = $derived(app.settings?.limit ? Math.round(app.settings.limit / 1e6) : 0);
  const peak = $derived(Math.max(5e6, ...app.history));
  const columns = $derived(
    Array.from({ length: COLUMNS }, (_, i) => {
      const v = app.history[app.history.length - COLUMNS + i] ?? 0;
      return Math.round((v / peak) * ROWS);
    }),
  );

  const FINISH: { value: FinishAction; label: string }[] = [
    { value: 'nothing', label: msg('Do nothing') },
    { value: 'quit', label: msg('Quit KlyxDown') },
    { value: 'sleep', label: msg('Sleep') },
    { value: 'shutdown', label: msg('Shut down') },
  ];

  function setFinish(action: FinishAction) {
    app.finishAction = action;
    app.run(() => api.setFinishAction(action));
  }

  function setMode(m: SpeedMode) {
    if (!app.settings) return;
    app.settings = { ...app.settings, mode: m };
    app.run(() => api.setMode(m));
  }

  function stepLimit(dir: number) {
    if (!app.settings) return;
    const i = LIMITS.findIndex((l) => l >= limitMb);
    const next = LIMITS[Math.max(0, Math.min(LIMITS.length - 1, (i < 0 ? 0 : i) + dir))]!;
    const limit = next ? next * 1e6 : null;
    app.settings = { ...app.settings, limit };
    app.run(() => api.setLimit(limit));
  }
</script>

<aside class="master panel" aria-label={t('Speed')}>
  <div>
    <Knob {mode} onchange={setMode} />
    <p class="mode-note">{t(NOTE[mode])}</p>
  </div>

  <div class="meter">
    <div class="total"><span class="label">{t('Total')}</span><b class="num">{mbps(app.total)}<small class="label"> {t('MB/s')}</small></b></div>
    <div class="vu" aria-hidden="true">
      {#each columns as lit, c (c)}
        <div>{#each Array(ROWS) as _, r (r)}<i class:on={ROWS - 1 - r < lit}></i>{/each}</div>
      {/each}
    </div>
  </div>

  <div class="limit">
    <span class="label">{t('Speed limit')}</span>
    <div class="stepper">
      <button class="key sm sq" type="button" aria-label={t('Lower the limit')} onclick={() => stepLimit(-1)} disabled={limitMb === 0}><Icon name="minus" size={14} /></button>
      <output class="num">{limitMb ? t('{n} MB/s', { n: limitMb }) : t('No limit')}</output>
      <button class="key sm sq" type="button" aria-label={t('Raise the limit')} onclick={() => stepLimit(1)} disabled={limitMb === LIMITS.at(-1)}><Icon name="plus" size={14} /></button>
    </div>
  </div>
  <div class="limit">
    <span class="label">{t('When everything finishes')}</span>
    <Select id="finish-action" label={t('When everything finishes')} value={app.finishAction} options={FINISH.map((o) => ({ ...o, label: t(o.label) }))} onchange={setFinish} />
  </div>
</aside>

<style>
  .master {
    grid-area: master;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow: auto;
  }
  .mode-note {
    margin: -8px 0 0;
    min-height: 2.8em;
    text-align: center;
    font-size: 12px;
    color: var(--ink-2);
  }
  .total {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .total b {
    font-size: 26px;
    font-weight: 650;
    font-stretch: 66%;
  }
  .vu {
    display: grid;
    grid-template-columns: repeat(18, 1fr);
    gap: 2px;
    height: 56px;
    padding: 4px;
    border-radius: 6px;
    background: var(--recess);
    box-shadow: inset 0 1px 2px var(--lo);
  }
  .vu div {
    display: grid;
    grid-template-rows: repeat(8, 1fr);
    gap: 2px;
  }
  .vu i {
    border-radius: 1px;
    background: var(--led-off);
    transition: background-color 0.2s;
  }
  .vu i.on {
    background: var(--accent);
  }
  .limit {
    display: grid;
    gap: 8px;
  }
  .stepper {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .stepper output {
    flex: 1;
    text-align: center;
    font-weight: 650;
    font-size: 14px;
  }
</style>
