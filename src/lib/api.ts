import { Channel, invoke } from '@tauri-apps/api/core';
import type { AddRequest, Captured, CapturedLinks, DownloadView, FinishAction, Id, Probe, Settings, SpeedMode, UpdateInfo } from './types';

// Thin typed wrappers around the Rust commands in src-tauri/src/commands.rs.
export const api = {
  probe: (url: string, headers: [string, string][] = []) => invoke<Probe>('probe', { url, headers }),
  add: (req: AddRequest) => invoke<Id>('add', { req }),
  suggestDir: (name: string) => invoke<string>('suggest_dir', { name }),
  sha256: (id: Id) => invoke<string>('sha256', { id }),
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
  uiReady: () => invoke<{ show: boolean; capture: Captured | null; links: CapturedLinks | null }>('ui_ready'),
  openFile: (id: Id) => invoke<void>('open_file', { id }),
  revealFile: (id: Id) => invoke<void>('reveal_file', { id }),
  quit: () => invoke<void>('quit'),
  watch(onList: (list: DownloadView[]) => void) {
    const channel = new Channel<DownloadView[]>();
    channel.onmessage = onList;
    return invoke<void>('watch', { onProgress: channel });
  },
};

/** Command errors arrive as plain strings (see AppError in commands.rs). */
export function errorText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong.';
}
