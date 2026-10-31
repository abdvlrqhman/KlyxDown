import type { DownloadView } from './types';
import type { IconName } from './icons';
import { currentLanguage, t } from './i18n.svelte';

const MB = 1e6;
const GB = 1e9;

export function bytes(b: number | null | undefined): string {
  if (b == null) return t('Unknown size');
  if (b >= GB) return t('{n} GB', { n: (b / GB).toFixed(2) });
  if (b >= MB) return t('{n} MB', { n: (b / MB).toFixed(1) });
  if (b >= 1e3) return t('{n} KB', { n: Math.round(b / 1e3) });
  return t('{n} B', { n: b });
}

export function speed(bps: number): string {
  return bps >= MB ? t('{n} MB/s', { n: (bps / MB).toFixed(1) }) : t('{n} KB/s', { n: Math.round(bps / 1e3) });
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
  if (h) return t('{h} h {m} min', { h, m });
  if (m) return t('{m} min {s} s', { m, s: String(s).padStart(2, '0') });
  return t('{s} s', { s });
}

export function pct(d: DownloadView): number {
  return d.size ? (d.bytes / d.size) * 100 : 0;
}

export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(currentLanguage(), { hour: '2-digit', minute: '2-digit' });
}

export function finishedText(ms: number | null): string {
  if (!ms) return t('Finished');
  const d = new Date(ms);
  const today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  return sameDay ? t('Finished at {time}', { time: clock(ms) }) : t('Finished {date}', { date: d.toLocaleDateString(currentLanguage()) });
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

/** Video and audio the preview player can try (the webview decides in the end). */
export const PLAYABLE = /\.(mp4|m4v|webm|mkv|mov|ogv|mp3|m4a|aac|ogg|opus|flac|wav)$/i;

export function kindIcon(name: string): IconName {
  return KINDS.find(([re]) => re.test(name))?.[1] ?? 'file';
}
