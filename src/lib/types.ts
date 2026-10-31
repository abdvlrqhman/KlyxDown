// Mirrors the serde types in crates/engine/src/lib.rs and src-tauri/src/settings.rs.

export type Id = number;
export type SpeedMode = 'snail' | 'balanced' | 'full';
export type DlState = 'queued' | 'downloading' | 'paused' | 'waitingForNetwork' | 'failed' | 'done';
export type Kind = 'direct';
export type QueueId = number;
export const MAIN_QUEUE: QueueId = 0;

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
  kind: Kind;
  queue: QueueId;
  /** The server refused the link; a fresh one from the page fixes it. */
  needsLink: boolean;
  mirrors: number;
  /** The page the download came from (its referrer). */
  page: string | null;
}

export type Algo = 'md5' | 'sha1' | 'sha256' | 'sha512';
export interface Checksum {
  algo: Algo;
  hex: string;
}

/** One file a Metalink describes (crates/engine/src/metalink.rs). */
export interface MetalinkFile {
  name: string;
  size: number | null;
  urls: string[];
  checksum: Checksum | null;
}

/** One entry of a remote ZIP (crates/engine/src/zip.rs). */
export interface ZipEntry {
  name: string;
  size: number;
  compressed: number;
  dir: boolean;
  encrypted: boolean;
  method: number;
}

/** A weekly window in local time; minutes after midnight, `days` bit 0 is Monday. */
export interface Schedule {
  start: number;
  stop: number;
  days: number;
}

export interface Queue {
  id: QueueId;
  name: string;
  maxActive: number;
  stopped: boolean;
  schedule: Schedule | null;
}

export type Hold = { kind: 'metered' } | { kind: 'quota'; resumesAt: number };

export interface Status {
  hold: Hold | null;
  quotaUsed: number;
}

/** What the progress stream sends 4 times a second. */
export interface Snapshot {
  downloads: DownloadView[];
  status: Status;
  metered: boolean;
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
  queue?: QueueId;
  mirrors?: string[];
  checksum?: Checksum | null;
  zipEntry?: string | null;
}

export type ThemeName = 'system' | 'midnight' | 'graphite' | 'fern' | 'aluminum' | 'sand';

export interface Settings {
  theme: ThemeName;
  language: string;
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
  pauseOnMetered: boolean;
  quotaMb: number | null;
  quotaHours: number;
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
  /** From a Metalink: names, mirrors and checksums for each link in `urls`. */
  files?: MetalinkFile[];
}
