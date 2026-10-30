// Mirrors the serde types in crates/engine/src/lib.rs and src-tauri/src/settings.rs.

export type Id = number;
export type SpeedMode = 'snail' | 'balanced' | 'full';
export type DlState = 'queued' | 'downloading' | 'paused' | 'waitingForNetwork' | 'failed' | 'done';

export interface SegmentView {
  start: number;
  end: number;
  pos: number;
  speed: number;
}

export interface DownloadView {
  id: Id;
  url: string;
  name: string;
  host: string;
  path: string;
  size: number | null;
  bytes: number;
  speed: number;
  state: DlState;
  error: string | null;
  resumable: boolean;
  connections: number;
  segments: SegmentView[];
  addedAt: number;
  finishedAt: number | null;
}

export interface Probe {
  url: string;
  name: string;
  host: string;
  size: number | null;
  resumable: boolean;
  mime: string | null;
}

export interface AddRequest {
  url: string;
  dir: string;
  fileName: string | null;
  connections: number;
  start: boolean;
  headers: [string, string][];
}

export type ThemeName = 'system' | 'midnight' | 'graphite' | 'fern' | 'aluminum' | 'sand';

export interface Settings {
  theme: ThemeName;
  runOnStartup: boolean;
  startHidden: boolean;
  minimizeToTray: boolean;
  closeToTray: boolean;
  downloadDir: string;
  maxActive: number;
  connections: number;
  mode: SpeedMode;
  limit: number | null;
  askOnCapture: boolean;
  sortByType: boolean;
  showIntro: boolean;
  onboarded: boolean;
  proxyMode: 'system' | 'none' | 'custom';
  proxyUrl: string;
  keepAwake: boolean;
  watchClipboard: boolean;
  autoUpdate: boolean;
}

export interface UpdateInfo {
  version: string;
  current: string;
  notes: string | null;
}

export type FinishAction = 'nothing' | 'quit' | 'sleep' | 'shutdown';

/** A download the browser extension handed over (src-tauri/src/bridge.rs). */
export interface Captured {
  /** Non-zero while the browser waits for a decision (call api.captureDone). */
  id: number;
  url: string;
  name: string | null;
  size: number | null;
  headers: [string, string][];
  browser: string;
}

export interface CapturedLinks {
  urls: string[];
  title: string | null;
  headers: [string, string][];
  browser: string;
}
