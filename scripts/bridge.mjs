// Builds the browser helper (crates/bridge) and puts it where Tauri expects
// sidecar binaries, so `tauri dev` and installers ship it next to the app.
// Usage: node scripts/bridge.mjs [--release]
//
// Tauri sets TAURI_ENV_TARGET_TRIPLE for before-build commands, so cross builds
// (for example Intel macOS on an Apple Silicon runner) get a matching helper.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';

const release = process.argv.includes('--release');
const host = /host: (\S+)/.exec(execFileSync('rustc', ['-vV']).toString())?.[1];
if (!host) throw new Error('Could not read the Rust host target from `rustc -vV`.');
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const cross = triple !== host;

const args = ['build', '-p', 'klyxdown-bridge'];
if (release) args.push('--release');
if (cross) args.push('--target', triple);
execFileSync('cargo', args, { stdio: 'inherit' });

const ext = triple.includes('windows') ? '.exe' : '';
const profile = release ? 'release' : 'debug';
const built = cross ? `target/${triple}/${profile}/klyxdown-bridge${ext}` : `target/${profile}/klyxdown-bridge${ext}`;
mkdirSync('src-tauri/binaries', { recursive: true });
copyFileSync(built, `src-tauri/binaries/klyxdown-bridge-${triple}${ext}`);
