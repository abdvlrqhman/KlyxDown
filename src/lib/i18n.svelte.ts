// Interface languages. English is the source text: `t('Pause all')` shows the English text
// unless the current language translates it, so a missing translation is never a blank.
// Plurals: pass `n`; the English text is the "other" form and locales/en.json holds the
// English "one" forms. Other languages give every form their plural rules use (Arabic has six).
import en from '../locales/en.json';

type Plural = Partial<Record<Intl.LDMLPluralRule, string>>;
export type Messages = Record<string, string | Plural>;

export interface Language {
  code: string;
  /** In the language itself, so people can find theirs. */
  name: string;
  dir: 'ltr' | 'rtl';
}

export const LANGUAGES: Language[] = [
  { code: 'en', name: 'English', dir: 'ltr' },
  { code: 'ar', name: 'العربية', dir: 'rtl' },
  { code: 'fa', name: 'فارسی', dir: 'rtl' },
  { code: 'de', name: 'Deutsch', dir: 'ltr' },
  { code: 'es', name: 'Español', dir: 'ltr' },
  { code: 'fr', name: 'Français', dir: 'ltr' },
  { code: 'id', name: 'Bahasa Indonesia', dir: 'ltr' },
  { code: 'it', name: 'Italiano', dir: 'ltr' },
  { code: 'ja', name: '日本語', dir: 'ltr' },
  { code: 'pt-BR', name: 'Português (Brasil)', dir: 'ltr' },
  { code: 'ru', name: 'Русский', dir: 'ltr' },
  { code: 'tr', name: 'Türkçe', dir: 'ltr' },
  { code: 'zh-CN', name: '简体中文', dir: 'ltr' },
];

const loaders = import.meta.glob<Messages>('../locales/*.json', { import: 'default' });

const source = en as Messages;
let lang = $state('en');
let messages = $state.raw<Messages>(source);
let rules = new Intl.PluralRules('en');

/** The best supported language for a setting: "system" follows the OS (through the webview). */
export function resolveLanguage(setting: string): Language {
  const wanted = setting === 'system' ? navigator.languages : [setting];
  for (const code of wanted) {
    const exact = LANGUAGES.find((l) => l.code.toLowerCase() === code.toLowerCase());
    const base = LANGUAGES.find((l) => l.code.split('-')[0] === code.split('-')[0]);
    if (exact ?? base) return (exact ?? base)!;
  }
  return LANGUAGES[0]!;
}

/** Loads the language's messages, then flips text direction for right-to-left languages. */
export async function setLanguage(setting: string) {
  const language = resolveLanguage(setting);
  const load = loaders[`../locales/${language.code}.json`];
  messages = language.code === 'en' || !load ? source : await load().catch(() => source);
  rules = new Intl.PluralRules(language.code);
  lang = language.code;
  document.documentElement.lang = language.code;
  document.documentElement.dir = language.dir;
}

export const currentLanguage = () => lang;

export function t(text: string, vars?: Record<string, string | number>): string {
  // Empty strings are untranslated slots in the language files.
  const entry = messages[text] || source[text];
  let out = text;
  if (typeof entry === 'string') out = entry;
  else if (entry) out = entry[rules.select(Number(vars?.n ?? 0))] ?? entry.other ?? text;
  return vars ? out.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : out;
}

/** Marks text defined away from where it is shown (lists, constants) for translation;
 *  show it with `t(text)`. scripts/i18n.mjs finds both. */
export const msg = (text: string) => text;
