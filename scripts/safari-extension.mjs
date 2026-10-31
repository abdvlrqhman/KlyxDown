// Builds the Safari version of the extension (macOS, Xcode 15+):
//   node scripts/safari-extension.mjs [--team TEAMID]
// 1. Copies extension/ to target/extension-safari without the Chrome-only `key`.
// 2. Wraps it with Apple's safari-web-extension-converter into target/safari (an Xcode project).
// 3. Puts in our native handler (safari/SafariWebExtensionHandler.swift), which relays
//    messages to KlyxDown over a socket in a shared App Group folder.
// Build and sign the project with Xcode or xcodebuild (see docs/RELEASING.md).
import { execFileSync } from 'node:child_process';
import { cpSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const team = process.argv.includes('--team') ? process.argv[process.argv.indexOf('--team') + 1] : process.env.APPLE_TEAM_ID;
// Keep in sync with GROUP_SUFFIX in src-tauri/src/bridge.rs.
const group = `${team ?? 'TEAMID'}.net.spacie.klyxdown`;
const ext = 'target/extension-safari';
rmSync(ext, { recursive: true, force: true });
cpSync('extension', ext, { recursive: true });
const manifest = JSON.parse(readFileSync(`${ext}/manifest.json`, 'utf8'));
delete manifest.key;
// Safari has neither of these; the extension checks before using them.
manifest.permissions = manifest.permissions.filter((p) => !['downloads', 'webRequest'].includes(p));
writeFileSync(`${ext}/manifest.json`, `${JSON.stringify(manifest, null, 2)}\n`);

rmSync('target/safari', { recursive: true, force: true });
execFileSync(
  'xcrun',
  ['safari-web-extension-converter', ext, '--project-location', 'target/safari', '--app-name', 'KlyxDown for Safari',
    '--bundle-identifier', 'net.spacie.klyxdown.safari', '--swift', '--macos-only', '--no-open', '--no-prompt', '--force'],
  { stdio: 'inherit' },
);

// Our handler and the App Group (so the sandboxed extension can reach KlyxDown's socket).
const root = join('target/safari', 'KlyxDown for Safari');
const walk = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
for (const file of walk(root)) {
  // The converter names the app after its title; the extension id must start with the app's.
  if (file.endsWith('project.pbxproj')) {
    writeFileSync(file, readFileSync(file, 'utf8').replaceAll('net.spacie.klyxdown.KlyxDown-for-Safari', 'net.spacie.klyxdown.safari'));
  }
  if (file.endsWith('SafariWebExtensionHandler.swift')) cpSync('safari/SafariWebExtensionHandler.swift', file);
  if (file.endsWith('.entitlements')) {
    const text = readFileSync(file, 'utf8');
    if (!text.includes('application-groups')) {
      writeFileSync(file, text.replace('<dict>', `<dict>\n\t<key>com.apple.security.application-groups</key>\n\t<array>\n\t\t<string>${group}</string>\n\t</array>`));
    }
  }
  if (file.endsWith('Info.plist') && file.includes('Extension')) {
    const text = readFileSync(file, 'utf8');
    if (!text.includes('KlyxDownAppGroup')) {
      writeFileSync(file, text.replace('<dict>', `<dict>\n\t<key>KlyxDownAppGroup</key>\n\t<string>${group}</string>`));
    }
  }
}
console.log(`Safari project written to ${root} (App Group ${group})`);
