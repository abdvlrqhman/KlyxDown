import type { ThemeName } from './types';

export interface ThemeInfo {
  name: ThemeName;
  label: string;
  note: string;
}

// Token values live in app.css under [data-theme="..."]; this is the picker list.
export const THEMES: ThemeInfo[] = [
  { name: 'system', label: 'Follow system', note: 'Midnight at night, Aluminum by day' },
  { name: 'midnight', label: 'Midnight', note: 'Deep blue, ice blue lights' },
  { name: 'graphite', label: 'Graphite', note: 'Warm black, amber lights' },
  { name: 'fern', label: 'Fern', note: 'Dark green, mint lights' },
  { name: 'aluminum', label: 'Aluminum', note: 'Cool light grey, blue lights' },
  { name: 'sand', label: 'Sand', note: 'Warm light, teal lights' },
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
