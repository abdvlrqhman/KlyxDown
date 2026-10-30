import type { DownloadView } from './types';
import type { IconName } from './icons';

const MB = 1e6;
const GB = 1e9;

export function bytes(b: number | null | undefined): string {
  if (b == null) return 'Unknown size';
  if (b >= GB) return `${(b / GB).toFixed(2)} GB`;
  if (b >= MB) return `${(b / MB).toFixed(1)} MB`;
  if (b >= 1e3) return `${Math.round(b / 1e3)} KB`;
  return `${b} B`;
}

export function speed(bps: number): string {
  return bps >= MB ? `${(bps / MB).toFixed(1)} MB/s` : `${Math.round(bps / 1e3)} KB/s`;
}

export function mbps(bps: number): string {
  return (bps / MB).toFixed(1);
}

export function eta(d: DownloadView): number {
  if (!d.size || d.speed <= 0) return Infinity;
  return (d.size - d.bytes) / d.speed;
}

export function etaText(sec: number): string {
  if (!Number.isFinite(sec)) return '';
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = Math.floor(sec % 60);
  if (h) return `${h} h ${m} min`;
  if (m) return `${m} min ${String(s).padStart(2, '0')} s`;
  return `${s} s`;
}

export function pct(d: DownloadView): number {
  return d.size ? (d.bytes / d.size) * 100 : 0;
}

export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

export function finishedText(ms: number | null): string {
  if (!ms) return 'Finished';
  const d = new Date(ms);
  const today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  return sameDay ? `Finished at ${clock(ms)}` : `Finished ${d.toLocaleDateString()}`;
}

const KINDS: [RegExp, IconName][] = [
  [/\.(iso|img|dmg|vhdx?)$/i, 'disc'],
  [/\.(exe|msi|msix|pkg|deb|rpm|appimage)$/i, 'app-window'],
  [/\.(mp4|mkv|webm|mov|avi|m4v)$/i, 'film-strip'],
  [/\.(zip|7z|rar|gz|xz|tar|bz2|zst)$/i, 'file-zip'],
  [/\.(mp3|flac|wav|ogg|m4a|aac|opus)$/i, 'music-notes'],
  [/\.(pdf|epub|docx?|txt|md|odt)$/i, 'file-text'],
  [/\.torrent$/i, 'magnet'],
];

export function kindIcon(name: string): IconName {
  return KINDS.find(([re]) => re.test(name))?.[1] ?? 'file';
}
