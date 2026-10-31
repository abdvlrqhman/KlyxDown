<script lang="ts">
  import { fade, fly } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import ScrollArea from './ScrollArea.svelte';
  import Select from './Select.svelte';
  import { api } from './api';
  import { currentLanguage, t } from './i18n.svelte';
  import { ms } from './motion';
  import { app } from './store.svelte';
  import { MAIN_QUEUE, type Queue, type Schedule } from './types';

  // Queues: each runs its own number of downloads, optionally only inside a weekly time window
  // (a night queue, say). Changes apply right away.

  const close = () => (app.queuesOpen = false);
  const EVERY_DAY = 0x7f;
  const NIGHT: Schedule = { start: 60, stop: 7 * 60, days: EVERY_DAY };

  // Half-hour steps, shown in the user's own clock format.
  const times = $derived(
    Array.from({ length: 48 }, (_, i) => ({
      value: i * 30,
      label: new Date(2000, 0, 1, 0, i * 30).toLocaleTimeString(currentLanguage(), { hour: '2-digit', minute: '2-digit' }),
    })),
  );
  // Monday first, matching the engine's day bits.
  const days = $derived(
    Array.from({ length: 7 }, (_, i) => new Date(2024, 0, 1 + i).toLocaleDateString(currentLanguage(), { weekday: 'short' })),
  );
  const count = (q: Queue) => app.downloads.filter((d) => d.queue === q.id && d.state !== 'done').length;

  async function save(q: Queue, change: Partial<Queue>) {
    const next = { ...q, ...change };
    app.queues = app.queues.map((x) => (x.id === q.id ? next : x));
    if (q.id === MAIN_QUEUE && change.maxActive && app.settings) app.settings = { ...app.settings, maxActive: change.maxActive };
    await app.run(() => api.updateQueue(next));
  }

  async function add(night: boolean) {
    const name = night ? t('Night') : t('Queue {n}', { n: app.queues.length });
    await app.run(() => api.addQueue({ id: 0, name, maxActive: 2, stopped: false, schedule: night ? NIGHT : null }));
    await app.loadQueues();
  }

  async function remove(q: Queue) {
    await app.run(() => api.removeQueue(q.id));
    await app.loadQueues();
    app.toast(t('Removed {name}. Its downloads moved to the main queue.', { name: q.name }));
  }

  function status(q: Queue) {
    if (q.stopped) return t('Stopped');
    if (!q.schedule) return t('Runs any time');
    const at = times.find((x) => x.value === q.schedule!.start)?.label ?? '';
    const until = times.find((x) => x.value === q.schedule!.stop)?.label ?? '';
    return q.schedule.start === q.schedule.stop ? t('All day on the days picked') : t('From {start} to {stop}', { start: at, stop: until });
  }
</script>

<div class="scrim" transition:fade={{ duration: ms(150) }} onclick={close} role="presentation"></div>
<div class="sheet panel" role="dialog" aria-modal="true" aria-labelledby="queues-title" transition:fly={{ x: document.dir === 'rtl' ? -40 : 40, duration: ms(280) }}>
  <header>
    <h2 id="queues-title">{t('Queues')}</h2>
    <button class="key sm sq" type="button" aria-label={t('Close')} onclick={close}><Icon name="x" /></button>
  </header>
  <p class="intro">{t('Each queue runs its own number of downloads. A schedule keeps a queue to certain hours, like a night queue for big files.')}</p>

  <ScrollArea class="queues-scroll">
    <div class="list">
      {#each app.queues as q (q.id)}
        <section class="card" aria-label={app.queueName(q.id)}>
          <div class="top">
            {#if q.id === MAIN_QUEUE}
              <b class="name">{app.queueName(q.id)}</b>
            {:else}
              <input class="slot name-input" value={q.name} aria-label={t('Queue name')} spellcheck="false" onchange={(e) => save(q, { name: e.currentTarget.value })} />
            {/if}
            <span class="count num">{t('{n} waiting or running', { n: count(q) })}</span>
          </div>

          <div class="line">
            <span>{t('Downloads at the same time')}</span>
            <div class="stepper">
              <button class="key sm sq" type="button" aria-label={t('Less')} disabled={q.maxActive <= 1} onclick={() => save(q, { maxActive: q.maxActive - 1 })}><Icon name="minus" size={14} /></button>
              <output class="num">{q.maxActive}</output>
              <button class="key sm sq" type="button" aria-label={t('More')} disabled={q.maxActive >= 16} onclick={() => save(q, { maxActive: q.maxActive + 1 })}><Icon name="plus" size={14} /></button>
            </div>
          </div>

          <div class="line">
            <span>{t('Only at certain times')}</span>
            <button class="switch" type="button" role="switch" aria-checked={!!q.schedule} aria-label={t('Only at certain times')} onclick={() => save(q, { schedule: q.schedule ? null : NIGHT })}></button>
          </div>

          {#if q.schedule}
            {@const sch = q.schedule}
            <div class="times">
              <label for="start-{q.id}">{t('From')}</label>
              <Select id="start-{q.id}" label={t('From')} value={sch.start} options={times} onchange={(v) => save(q, { schedule: { ...sch, start: v } })} />
              <label for="stop-{q.id}">{t('Until')}</label>
              <Select id="stop-{q.id}" label={t('Until')} value={sch.stop} options={times} onchange={(v) => save(q, { schedule: { ...sch, stop: v } })} />
            </div>
            <div class="days" role="group" aria-label={t('Days')}>
              {#each days as day, i (i)}
                <button class="key sm" type="button" aria-pressed={(sch.days & (1 << i)) !== 0} onclick={() => save(q, { schedule: { ...sch, days: sch.days ^ (1 << i) } })}>{day}</button>
              {/each}
            </div>
          {/if}

          <div class="foot">
            <span class="status" class:off={q.stopped}><Icon name={q.schedule ? 'calendar-blank' : 'queue'} size={14} />{status(q)}</span>
            <button class="key sm" type="button" onclick={() => save(q, { stopped: !q.stopped })}>
              <Icon name={q.stopped ? 'play' : 'stop'} />{q.stopped ? t('Start queue') : t('Stop queue')}
            </button>
            {#if q.id !== MAIN_QUEUE}
              <button class="key sm sq" type="button" aria-label={t('Remove queue')} title={t('Remove queue')} onclick={() => remove(q)}><Icon name="trash" /></button>
            {/if}
          </div>
        </section>
      {/each}
    </div>
  </ScrollArea>

  <footer>
    <button class="key sm" type="button" onclick={() => add(true)}><Icon name="moon" />{t('Add a night queue')}</button>
    <button class="key sm go" type="button" onclick={() => add(false)}><Icon name="plus" />{t('Add a queue')}</button>
  </footer>
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
    padding: 14px 14px 6px 18px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 16px;
    font-stretch: 95%;
  }
  .intro {
    margin: 0;
    padding: 0 18px 12px;
    font-size: 12px;
    color: var(--ink-2);
  }
  .sheet :global(.queues-scroll) {
    flex: 1;
  }
  .list {
    display: grid;
    gap: 10px;
    padding: 0 18px 12px;
  }
  .card {
    display: grid;
    gap: 12px;
    padding: 12px;
    border-radius: 8px;
    background: var(--chassis);
    box-shadow: inset 0 1px 2px var(--lo);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .name {
    flex: 1;
    font-weight: 650;
  }
  .name-input {
    flex: 1;
    min-width: 0;
    font-weight: 650;
  }
  .count {
    font-size: 12px;
    color: var(--ink-2);
    white-space: nowrap;
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
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
  .times {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    gap: 8px;
  }
  .times label {
    font-size: 12px;
    color: var(--ink-2);
  }
  .days {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .days .key {
    flex: 1;
    min-width: 0;
    padding-inline: 0;
  }
  .days .key[aria-pressed='true'] {
    background: var(--accent);
    color: var(--on-accent);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--ink-2);
  }
  .status.off {
    color: var(--warn);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px 16px;
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
</style>
