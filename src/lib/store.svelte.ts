import { api, errorText } from './api';
import { setLanguage, t } from './i18n.svelte';
import { applyTheme } from './themes';
import type { Captured, CapturedLinks, CapturedMedia, DownloadView, FinishAction, Queue, Settings, Snapshot, Status, UpdateInfo } from './types';

export interface Toast {
  id: number;
  text: string;
}

const HISTORY = 24;

/** All UI state in one place. The engine is the source of truth for downloads. */
class AppState {
  downloads = $state.raw<DownloadView[]>([]);
  status = $state.raw<Status>({ hold: null, quotaUsed: 0 });
  metered = $state(false);
  queues = $state.raw<Queue[]>([]);
  queuesOpen = $state(false);
  /** Search text (Ctrl+F); filters every list by name, site and link. */
  query = $state('');
  settings = $state<Settings | null>(null);
  online = $state(navigator.onLine);
  settingsOpen = $state(false);
  addOpen = $state(false);
  addPrefill = $state('');
  /** Bumped to remount the add panel when a new link arrives while it is open. */
  addKey = $state(0);
  /** Set when the add panel was opened by the browser extension. */
  capture = $state<Captured | null>(null);
  links = $state<CapturedLinks | null>(null);
  /** Chosen in the Master panel; resets every launch. `countdown` is set when it is about to run. */
  finishAction = $state<FinishAction>('nothing');
  countdown = $state<FinishAction | null>(null);
  /** A newer version found on GitHub, and download progress while installing it. */
  update = $state<UpdateInfo | null>(null);
  updating = $state<{ received: number; total: number | null } | null>(null);
  /** Launch intro and first-run guide. */
  intro = $state(false);
  onboarding = $state(false);
  /** Finished download whose checksum dialog is open. */
  verify = $state<DownloadView | null>(null);
  /** A remote ZIP whose files are being picked. */
  zip = $state<{ url: string; name: string; headers: [string, string][] } | null>(null);
  /** A download whose link expired, waiting for a fresh one. */
  refresh = $state<DownloadView | null>(null);
  /** A web page whose video is being picked. */
  media = $state<{ url: string; headers: [string, string][] } | null>(null);
  /** A magnet or `.torrent` whose files are being picked. */
  torrent = $state<{ source: string } | null>(null);
  /** A download playing in the preview player. */
  preview = $state<DownloadView | null>(null);
  /** A finished file being converted. */
  convert = $state<DownloadView | null>(null);
  toasts = $state<Toast[]>([]);
  /** Total speed samples for the meter, newest last. */
  history = $state.raw<number[]>([]);
  /** Ids that just finished; channels play their finish moment, then this clears. */
  justFinished = $state.raw<Set<number>>(new Set());

  total = $derived(this.downloads.reduce((sum, d) => sum + d.speed, 0));
  shown = $derived.by(() => {
    const words = this.query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return this.downloads;
    return this.downloads.filter((d) => {
      const text = `${d.name} ${d.host} ${d.url}`.toLowerCase();
      return words.every((w) => text.includes(w));
    });
  });
  needsYou = $derived(this.shown.filter((d) => d.state === 'failed'));
  running = $derived(this.shown.filter((d) => d.state === 'downloading' || d.state === 'waitingForNetwork'));
  waiting = $derived(this.shown.filter((d) => d.state === 'queued'));
  paused = $derived(this.shown.filter((d) => d.state === 'paused'));
  finished = $derived(this.shown.filter((d) => d.state === 'done').reverse());

  private toastId = 0;
  private saveTimer: ReturnType<typeof setTimeout> | undefined;

  setSnapshot(s: Snapshot) {
    this.status = s.status;
    this.metered = s.metered;
    this.setList(s.downloads);
  }

  async loadQueues() {
    this.queues = await api.queues();
  }

  queueName(id: number) {
    const q = this.queues.find((q) => q.id === id);
    return !q || id === 0 ? t('Main queue') : q.name;
  }

  setList(list: DownloadView[]) {
    const before = new Map(this.downloads.map((d) => [d.id, d.state]));
    const finishedNow = list.filter((d) => d.state === 'done' && before.has(d.id) && before.get(d.id) !== 'done');
    if (finishedNow.length) {
      this.justFinished = new Set([...this.justFinished, ...finishedNow.map((d) => d.id)]);
      finishedNow.forEach((d) => this.toast(t('Finished {name}', { name: d.name })));
    }
    this.downloads = list;
    this.history = [...this.history, this.total].slice(-HISTORY);
  }

  clearFinished(id: number) {
    const next = new Set(this.justFinished);
    next.delete(id);
    this.justFinished = next;
  }

  async loadSettings() {
    this.settings = await api.getSettings();
    applyTheme(this.settings.theme);
    await setLanguage(this.settings.language);
  }

  /** Applies a change immediately and saves it shortly after (several quick toggles, one write). */
  updateSettings(change: Partial<Settings>) {
    if (!this.settings) return;
    this.settings = { ...this.settings, ...change };
    if (change.theme) applyTheme(change.theme);
    if (change.language) setLanguage(change.language);
    clearTimeout(this.saveTimer);
    const snapshot = $state.snapshot(this.settings);
    this.saveTimer = setTimeout(async () => {
      try {
        this.settings = await api.saveSettings(snapshot);
      } catch (e) {
        this.toast(errorText(e));
        await this.loadSettings();
      }
    }, 250);
  }

  openAdd(prefill = '', capture: Captured | null = null) {
    // A new hand-off replaces the open one; the old one goes back to its browser.
    if (this.capture && this.capture.id !== capture?.id) this.giveBack(this.capture);
    this.capture = capture;
    this.addPrefill = prefill;
    this.addKey++;
    this.addOpen = true;
  }

  /** Closes the add panel. Unless the download was taken, the browser resumes its own copy. */
  closeAdd(taken = false) {
    if (this.capture && !taken) this.giveBack(this.capture);
    this.capture = null;
    this.addOpen = false;
  }

  private giveBack(c: Captured) {
    if (c.id) api.captureDone(c.id, false).catch(() => {});
  }

  /** From the extension: one file it played goes to the add panel, a page to the video picker. */
  takeMedia(m: CapturedMedia) {
    if (m.url) this.openAdd(m.url, { id: 0, url: m.url, name: null, size: null, headers: m.headers, browser: m.browser });
    else this.media = { url: m.page, headers: m.headers };
  }

  toast(text: string) {
    const id = ++this.toastId;
    this.toasts = [...this.toasts, { id, text }].slice(-3);
    setTimeout(() => (this.toasts = this.toasts.filter((t) => t.id !== id)), 3200);
  }

  /** Runs a command and shows its error as a toast instead of throwing. */
  async run(fn: () => Promise<unknown>) {
    try {
      await fn();
    } catch (e) {
      this.toast(errorText(e));
    }
  }
}

export const app = new AppState();
