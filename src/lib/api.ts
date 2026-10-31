import { Channel, invoke } from '@tauri-apps/api/core';
import type { AddRequest, FolderEntry, GrabOptions, Algo, Captured, CapturedLinks, CapturedMedia, FinishAction, Id, MediaInfo, MetalinkFile, Probe, Queue, QueueId, Settings, Snapshot, SpeedMode, ToolStatus, TorrentInfo, UpdateInfo, ZipEntry } from './types';

/** Tray menu and tooltip text; `busy` has {n} and {speed}. */
export interface TrayText {
  show: string;
  pauseAll: string;
  resumeAll: string;
  quit: string;
  idle: string;
  busy: string;
  offline: string;
}

// Thin typed wrappers around the Rust commands in src-tauri/src/commands.rs.
export const api = {
  probe: (url: string, headers: [string, string][] = []) => invoke<Probe>('probe', { url, headers }),
  add: (req: AddRequest) => invoke<Id>('add', { req }),
  suggestDir: (name: string) => invoke<string>('suggest_dir', { name }),
  fileHash: (id: Id, algo: Algo) => invoke<string>('file_hash', { id, algo }),
  /** The links a `[001-120]` pattern stands for; empty for a plain link. */
  expandBatch: (pattern: string) => invoke<string[]>('expand_batch', { pattern }),
  /** A Metalink file path or link. */
  readMetalink: (source: string) => invoke<MetalinkFile[]>('read_metalink', { source }),
  listZip: (url: string, headers: [string, string][] = []) => invoke<ZipEntry[]>('list_zip', { url, headers }),
  refreshLink: (id: Id, url: string) => invoke<void>('refresh_link', { id, url }),
  saveSite: (host: string, user: string, password: string) => invoke<Settings>('save_site', { host, user, password }),
  removeSite: (host: string) => invoke<Settings>('remove_site', { host }),
  listFolder: (url: string) => invoke<FolderEntry[]>('list_folder', { url }),
  grab: (url: string, options: GrabOptions) => invoke<string[]>('grab', { url, options }),
  stopGrab: () => invoke<void>('stop_grab'),
  toolsStatus: () => invoke<ToolStatus>('tools_status'),
  installTools: () => invoke<ToolStatus>('install_tools'),
  mediaInfo: (url: string, headers: [string, string][] = []) => invoke<MediaInfo>('media_info', { url, headers }),
  /** A magnet, a `.torrent` link or a `.torrent` file path. */
  inspectTorrent: (source: string) => invoke<TorrentInfo>('inspect_torrent', { source }),
  convertPresets: () => invoke<string[]>('convert_presets'),
  convert: (id: Id, preset: string) => invoke<Id>('convert', { id, preset }),
  setSeeding: (id: Id, seed: boolean) => invoke<void>('set_seeding', { id, seed }),
  previewUrl: (id: Id) => invoke<string>('preview_url', { id }),
  waitForLink: (id: Id) => invoke<void>('wait_for_link', { id }),
  importLinks: (path: string) => invoke<string[]>('import_links', { path }),
  captureDone: (id: number, taken: boolean) => invoke<void>('capture_done', { id, taken }),
  extensionSeen: () => invoke<boolean>('extension_seen'),
  openProjectPage: () => invoke<void>('open_project_page'),
  setFinishAction: (action: FinishAction) => invoke<FinishAction>('set_finish_action', { action }),
  runFinishAction: (action: FinishAction) => invoke<void>('run_finish_action', { action }),
  isPortable: () => invoke<boolean>('is_portable'),
  checkUpdate: () => invoke<UpdateInfo | null>('check_update'),
  installUpdate: () => invoke<void>('install_update'),
  pause: (id: Id) => invoke<void>('pause', { id }),
  resume: (id: Id) => invoke<void>('resume', { id }),
  restart: (id: Id) => invoke<void>('restart', { id }),
  remove: (id: Id, deleteFile = false) => invoke<void>('remove', { id, deleteFile }),
  pauseAll: () => invoke<void>('pause_all'),
  resumeAll: () => invoke<void>('resume_all'),
  setMode: (mode: SpeedMode) => invoke<void>('set_mode', { mode }),
  setLimit: (limit: number | null) => invoke<void>('set_limit', { limit }),
  networkChanged: (online: boolean) => invoke<void>('network_changed', { online }),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  /** Call once the UI is painted; the window is only shown after this. */
  uiReady: () => invoke<{ show: boolean; capture: Captured | null; links: CapturedLinks | null; media: CapturedMedia | null }>('ui_ready'),
  openFile: (id: Id) => invoke<void>('open_file', { id }),
  revealFile: (id: Id) => invoke<void>('reveal_file', { id }),
  quit: () => invoke<void>('quit'),
  queues: () => invoke<Queue[]>('queues'),
  addQueue: (queue: Queue) => invoke<QueueId>('add_queue', { queue }),
  updateQueue: (queue: Queue) => invoke<void>('update_queue', { queue }),
  removeQueue: (id: QueueId) => invoke<void>('remove_queue', { id }),
  moveToQueue: (id: Id, queue: QueueId) => invoke<void>('move_to_queue', { id, queue }),
  allowMetered: () => invoke<void>('allow_metered'),
  setTrayText: (text: TrayText) => invoke<void>('set_tray_text', { text }),
  watch(onSnapshot: (snapshot: Snapshot) => void) {
    const channel = new Channel<Snapshot>();
    channel.onmessage = onSnapshot;
    return invoke<void>('watch', { onProgress: channel });
  },
};

/** Command errors arrive as plain strings (see AppError in commands.rs). */
export function errorText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong.';
}
