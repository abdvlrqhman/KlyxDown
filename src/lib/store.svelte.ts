import { api, errorText } from './api';
import { applyTheme } from './themes';
import type { Captured, CapturedLinks, DownloadView, FinishAction, Settings, UpdateInfo } from './types';

export interface Toast {
  id: number;
  text: string;
}

const HISTORY = 24;

/** All UI state in one place. The engine is the source of truth for downloads. */
class AppState {
  downloads = $state.raw<DownloadView[]>([]);
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
  toasts = $state<Toast[]>([]);
  /** Total speed samples for the meter, newest last. */
  history = $state.raw<number[]>([]);
  /** Ids that just finished; channels play their finish moment, then this clears. */
  justFinished = $state.raw<Set<number>>(new Set());

  total = $derived(this.downloads.reduce((sum, d) => sum + d.speed, 0));
  needsYou = $derived(this.downloads.filter((d) => d.state === 'failed'));
  running = $derived(this.downloads.filter((d) => d.state === 'downloading' || d.state === 'waitingForNetwork'));
  waiting = $derived(this.downloads.filter((d) => d.state === 'queued'));
  paused = $derived(this.downloads.filter((d) => d.state === 'paused'));
  finished = $derived(this.downloads.filter((d) => d.state === 'done').reverse());

  private toastId = 0;
  private saveTimer: ReturnType<typeof setTimeout> | undefined;

  setList(list: DownloadView[]) {
    const before = new Map(this.downloads.map((d) => [d.id, d.state]));
    const finishedNow = list.filter((d) => d.state === 'done' && before.has(d.id) && before.get(d.id) !== 'done');
    if (finishedNow.length) {
      this.justFinished = new Set([...this.justFinished, ...finishedNow.map((d) => d.id)]);
      finishedNow.forEach((d) => this.toast(`Finished ${d.name}`));
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
  }

  /** Applies a change immediately and saves it shortly after (several quick toggles, one write). */
  updateSettings(change: Partial<Settings>) {
    if (!this.settings) return;
    this.settings = { ...this.settings, ...change };
    if (change.theme) applyTheme(change.theme);
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
