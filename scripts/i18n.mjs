// Interface text: finds every t('...') and msg('...') in src/, then checks each language file
// in src/locales against it.
//   node scripts/i18n.mjs           report what each language is missing
//   node scripts/i18n.mjs --write   also add missing texts (empty, for translators) and drop unused ones
// Fails when a translation uses a {placeholder} its English text does not have.
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const write = process.argv.includes('--write');
const files = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? files(join(dir, e.name)) : /\.(svelte|ts)$/.test(e.name) ? [join(dir, e.name)] : [],
  );

const texts = new Set();
const call = /\b(?:t|msg)\(\s*(['"])((?:\\.|(?!\1).)*)\1/g;
for (const file of files('src')) {
  for (const [, , raw] of readFileSync(file, 'utf8').matchAll(call)) texts.add(raw.replace(/\\(.)/g, '$1'));
}

const holes = (s) => new Set([...String(s).matchAll(/\{(\w+)\}/g)].map((m) => m[1]));
let bad = 0;
for (const name of readdirSync('src/locales').filter((f) => f.endsWith('.json') && f !== 'en.json').sort()) {
  const path = join('src/locales', name);
  const messages = JSON.parse(readFileSync(path, 'utf8'));
  const missing = [...texts].filter((k) => !messages[k]);
  const unused = Object.keys(messages).filter((k) => !texts.has(k));
  for (const [key, value] of Object.entries(messages)) {
    const forms = typeof value === 'string' ? [value] : Object.values(value);
    for (const form of forms.filter(Boolean)) {
      const extra = [...holes(form)].filter((h) => !holes(key).has(h));
      if (extra.length) {
        bad++;
        console.error(`${name}: "${key}" uses {${extra.join('}, {')}}, which the English text does not have`);
      }
    }
  }
  console.log(`${name}: ${texts.size - missing.length}/${texts.size} translated${unused.length ? `, ${unused.length} unused` : ''}`);
  if (write) {
    const next = Object.fromEntries([...texts].sort().map((k) => [k, messages[k] ?? '']));
    writeFileSync(path, `${JSON.stringify(next, null, 2)}\n`);
  }
}
console.log(`${texts.size} texts in the interface`);
if (bad) process.exit(1);
