// Dev-only: lets `npm run dev` render in a normal browser with sample downloads,
// so the UI can be styled without the Rust side. Never bundled in production
// (main.ts only imports it when import.meta.env.DEV and not inside Tauri).
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import type { DownloadView, Queue, Settings, Snapshot } from './types';

const MB = 1e6;
let settings: Settings = {
  theme: 'midnight',
  // ?lang=ar previews a language.
  language: new URLSearchParams(location.search).get('lang') ?? 'system',
  runOnStartup: false,
  startHidden: true,
  minimizeToTray: true,
  closeToTray: true,
  downloadDir: 'C:\\Users\\you\\Downloads',
  maxActive: 4,
  connections: 8,
  mode: 'balanced',
  limit: null,
  askOnCapture: true,
  sortByType: true,
  showIntro: true,
  onboarded: new URLSearchParams(location.search).has('lang'),
  proxyMode: 'system',
  proxyUrl: '',
  keepAwake: true,
  watchClipboard: false,
  autoUpdate: true,
  pauseOnMetered: false,
  quotaMb: null,
  quotaHours: 1,
  seed: true,
  seedRatio: 1,
  handleMagnets: true,
  scanAfter: true,
  scanner: '',
  sites: [{ host: 'files.example.com', user: 'me' }],
  remote: false,
  remotePort: 47950,
  remoteToken: '',
};
let queues: Queue[] = [
  { id: 0, name: 'Main', maxActive: 4, stopped: false, schedule: null },
  { id: 1, name: 'Night', maxActive: 2, stopped: false, schedule: { start: 60, stop: 420, days: 0x7f } },
];

function sample(id: number, name: string, host: string, sizeMb: number, done: number, conns: number, state: DownloadView['state']): DownloadView {
  const size = sizeMb * MB;
  const segments = Array.from({ length: conns }, (_, i) => {
    const start = Math.round((size * i) / conns);
    const end = Math.round((size * (i + 1)) / conns);
    const f = Math.min(1, Math.max(0.05, done + Math.sin(i * 1.7) * 0.25));
    return { start, end, pos: Math.round(start + (end - start) * f), speed: 0 };
  });
  return {
    id, url: `https://${host}/${name}`, name, host, path: `${settings.downloadDir}\\${name}`, size,
    bytes: segments.reduce((a, s) => a + s.pos - s.start, 0), speed: 0, state, error: null,
    resumable: true, connections: conns, segments, addedAt: Date.now(), finishedAt: state === 'done' ? Date.now() - 3.6e6 : null,
    kind: 'direct' as DownloadView['kind'], queue: 0, needsLink: state === 'failed', mirrors: 0, page: state === 'failed' ? 'https://drive.google.com/file/d/x/view' : null,
    detail: null, uploadSpeed: 0, uploaded: 0, peers: 0, seeding: false,
  };
}

const list: DownloadView[] = [
  sample(1, 'ubuntu-26.04-desktop-amd64.iso', 'releases.ubuntu.com', 6130, 0.71, 8, 'downloading'),
  sample(2, 'nvidia-studio-581.57-win11.exe', 'us.download.nvidia.com', 812, 0.41, 6, 'downloading'),
  sample(3, 'node-v26.0.0-x64.msi', 'nodejs.org', 31, 0.3, 4, 'downloading'),
  { ...sample(4, 'GoPro_2026-10-28_export.zip', 'drive.usercontent.google.com', 4710, 0.56, 8, 'failed'), error: 'The link expired or was refused (403). Get a fresh link from the page.' },
  sample(5, 'obs-studio-32.0.2-windows-x64.zip', 'github.com', 178, 0, 8, 'queued'),
  sample(6, 'DaVinci_Resolve_20.2_Windows.zip', 'swr.cloud.blackmagicdesign.com', 3870, 0.38, 8, 'paused'),
  sample(7, 'LibreOffice_26.8.2_Win_x86-64.msi', 'download.documentfoundation.org', 356, 1, 1, 'done'),
  { ...sample(8, 'debian-13.2.0-amd64-DVD-1.iso', '', 3900, 0.35, 1, 'downloading'), kind: 'torrent', peers: 23, uploadSpeed: 410e3 },
  { ...sample(9, 'Big Buck Bunny [aqz-KE-bpKQ].mp4', 'www.youtube.com', 412, 0.6, 1, 'downloading'), kind: 'media', detail: null },
  { ...sample(10, 'ubuntu-26.04-live-server-amd64.iso', '', 2100, 1, 1, 'done'), kind: 'torrent', seeding: true, uploadSpeed: 1.2e6, uploaded: 840e6 },
];

function tick() {
  for (const d of list) {
    if (d.state !== 'downloading' || !d.size) continue;
    let total = 0;
    for (const s of d.segments) {
      if (s.pos >= s.end) { s.speed = 0; continue; }
      s.speed = (0.6 + Math.random() * 0.8) * 1.2 * MB;
      s.pos = Math.min(s.end, s.pos + s.speed / 4);
      total += s.speed;
    }
    d.speed = total;
    d.bytes = d.segments.reduce((a, s) => a + s.pos - s.start, 0);
    if (d.bytes >= d.size) { d.state = 'done'; d.speed = 0; d.finishedAt = Date.now(); }
  }
}

export function installPreview() {
  mockWindows('main');
  mockIPC((cmd, args) => {
    const a = args as Record<string, unknown>;
    switch (cmd) {
      case 'get_settings': return settings;
      case 'save_settings': return (settings = a.settings as Settings);
      case 'suggest_dir': return `${settings.downloadDir}\Archives`;
      case 'file_hash': return '9f2c41e80b7a5d1c6e3f8a2b4c7d9e0f1a3b5c7d9e1f3a5b7c9d1e3f5a7b9c1d';
      case 'extension_seen': return false;
      case 'set_finish_action': return a.action;
      case 'is_portable': return false;
      case 'check_update': return null;
      case 'ui_ready': return { show: true, capture: null, links: null };
      case 'watch': {
        const ch = a.onProgress as { onmessage: (s: Snapshot) => void };
        setInterval(() => { tick(); ch.onmessage({ downloads: structuredClone(list), status: { hold: null, quotaUsed: 0 }, metered: false }); }, 250);
        return null;
      }
      case 'probe': return { url: a.url, name: String(a.url).split('/').pop() || 'download', host: new URL(String(a.url)).host, size: 344 * MB, resumable: true, mime: null };
      case 'expand_batch': return String(a.pattern).includes('[') ? Array.from({ length: 12 }, (_, i) => String(a.pattern).replace(/\[.*?\]/, String(i + 1).padStart(3, '0'))) : [];
      case 'list_zip': return [
        { name: 'docs/', size: 0, compressed: 0, dir: true, encrypted: false, method: 0 },
        { name: 'docs/manual.pdf', size: 4.2 * MB, compressed: 3.9 * MB, dir: false, encrypted: false, method: 8 },
        { name: 'setup.exe', size: 88 * MB, compressed: 80 * MB, dir: false, encrypted: false, method: 8 },
        { name: 'secret.key', size: 2e3, compressed: 2e3, dir: false, encrypted: true, method: 8 },
      ];
      case 'tools_status': return { ytdlp: '2026.10.24', ffmpeg: '7.1', ffmpegDownloadable: true };
      case 'convert_presets': return ['mp4', 'mp4-copy', 'mkv-copy', 'mp3', 'm4a', 'webm'];
      case 'media_info': return { title: 'Big Buck Bunny', site: 'Youtube', thumbnail: null, duration: 596, live: false, choices: [
        { label: '2160p60', format: 'x', audio: null, size: 1.2e9, height: 2160 }, { label: '1080p', format: 'x', audio: null, size: 412e6, height: 1080 },
        { label: '720p', format: 'x', audio: null, size: 180e6, height: 720 }, { label: 'Audio (M4A)', format: 'x', audio: 'm4a', size: 9e6, height: null }, { label: 'Audio (MP3)', format: 'x', audio: 'mp3', size: 9e6, height: null } ] };
      case 'inspect_torrent': return { name: 'debian-13.2.0-amd64', infoHash: 'ab', size: 4.1e9, files: [
        { index: 0, path: 'debian-13.2.0-amd64-DVD-1.iso', size: 3.9e9 }, { index: 1, path: 'SHA256SUMS', size: 1e3 }, { index: 2, path: 'README.txt', size: 4e3 } ] };
      case 'queues': return queues;
      case 'update_queue': queues = queues.map((q) => (q.id === (a.queue as Queue).id ? (a.queue as Queue) : q)); return null;
      case 'add_queue': queues = [...queues, { ...(a.queue as Queue), id: queues.length }]; return queues.length - 1;
      case 'remove_queue': queues = queues.filter((q) => q.id !== a.id); return null;
      case 'remove': {
        const i = list.findIndex((x) => x.id === a.id);
        if (i >= 0) list.splice(i, 1);
        return null;
      }
      case 'pause': case 'resume': {
        const d = list.find((x) => x.id === a.id);
        if (d) d.state = cmd === 'pause' ? 'paused' : 'downloading';
        return null;
      }
      default: return null;
    }
  }, { shouldMockEvents: true });
}
