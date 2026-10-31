import { msg } from './i18n.svelte';
import type { ThemeName } from './types';

export interface ThemeInfo {
  name: ThemeName;
  label: string;
  note: string;
}

// Token values live in app.css under [data-theme="..."]; this is the picker list.
export const THEMES: ThemeInfo[] = [
  { name: 'system', label: msg('Follow system'), note: msg('Midnight at night, Aluminum by day') },
  { name: 'midnight', label: msg('Midnight'), note: msg('Deep blue, ice blue lights') },
  { name: 'graphite', label: msg('Graphite'), note: msg('Warm black, amber lights') },
  { name: 'fern', label: msg('Fern'), note: msg('Dark green, mint lights') },
  { name: 'aluminum', label: msg('Aluminum'), note: msg('Cool light grey, blue lights') },
  { name: 'sand', label: msg('Sand'), note: msg('Warm light, teal lights') },
];

const dark = matchMedia('(prefers-color-scheme: dark)');

export function resolveTheme(name: ThemeName): Exclude<ThemeName, 'system'> {
  if (name !== 'system') return name;
  return dark.matches ? 'midnight' : 'aluminum';
}

export function applyTheme(name: ThemeName) {
  document.documentElement.dataset.theme = resolveTheme(name);
}

export function onSystemThemeChange(fn: () => void) {
  dark.addEventListener('change', fn);
  return () => dark.removeEventListener('change', fn);
}
