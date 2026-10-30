// extension/ is the Chrome, Edge and Brave build. Firefox needs two differences that
// Chrome warns about: background code as `background.scripts`, and a gecko id.
// This writes the Firefox build to target/extension-firefox.
// Usage: node scripts/firefox-extension.mjs
import { cpSync, readFileSync, rmSync, writeFileSync } from 'node:fs';

const out = 'target/extension-firefox';
rmSync(out, { recursive: true, force: true });
cpSync('extension', out, { recursive: true });

const manifest = JSON.parse(readFileSync('extension/manifest.json', 'utf8'));
delete manifest.key; // Chrome-only; fixes the Chrome extension id
manifest.background = { scripts: [manifest.background.service_worker] };
// Keep in sync with FIREFOX_EXTENSION_ID in src-tauri/src/bridge.rs.
manifest.browser_specific_settings = { gecko: { id: 'klyxdown@spacie.net', strict_min_version: '128.0' } };
writeFileSync(`${out}/manifest.json`, `${JSON.stringify(manifest, null, 2)}\n`);

console.log(`Firefox extension written to ${out}`);
