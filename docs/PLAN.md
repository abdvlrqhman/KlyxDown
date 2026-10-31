# KlyxDown: project plan

An open-source, cross-platform download manager built with Tauri 2. The goal is every feature people use in Internet Download Manager (IDM) and Free Download Manager (FDM), on Windows, macOS and Linux, with a calmer UI, no ads, no nags and no telemetry.

Status (2026-10-31): M0 to M5 done. Waiting only on accounts: code signing certificates and the store listings (see [RELEASING.md](RELEASING.md)). UI style: "Tactile" with the Midnight theme.

| Area | Done | Next |
|---|---|---|
| Shell | Custom title bar, tray (translated), run on startup, single instance, window state, 6 themes, 13 languages with right-to-left layout, launch intro, first-run guide, portable mode, signed auto-update, keep awake on Windows, macOS and Linux, phone remote (private networks, secret link, QR code) | Code signing certificates (Azure Trusted Signing, Apple Developer ID): workflow ready |
| Engine | Segmented HTTP and FTP/FTPS with dynamic splitting, slow connection replacement, mirrors, Metalink, MD5/SHA-1/SHA-256/SHA-512 checks, batch patterns, one file from a remote ZIP, fresh links for expired downloads, Basic and Digest sign-ins, queues with weekly schedules, hourly quota, metered hold, data flushed before progress is saved, requeue when the limit drops, antivirus check, Mark-of-the-Web and macOS quarantine | SFTP, proxy PAC files |
| Media | yt-dlp and ffmpeg installed on first use (checksums verified, daily yt-dlp update), quality picker, merged audio and video, audio-only, conversions (MP4, MKV, WebM, MP3, M4A), watch while downloading (seeking fetches ahead) | |
| Torrents | librqbit: magnets, `.torrent` files and links, file picker, sharing up to a ratio, streaming preview, magnet and `.torrent` handler | Per-torrent bandwidth |
| Browser | MV3 extension (Chrome, Edge, Brave, Opera, Vivaldi, Firefox, Safari wrapper): takeover with cookies, images left alone, menus, video detection, fresh-link pickup; built-in browser window | Store listings (texts ready, accounts needed) |
| Organize | Queues, scheduler, search, site grabber (robots.txt), site explorer (web listings, FTP), link list and Metalink import, clipboard watching | |
| Quality | Rust unit and integration tests (local HTTP, FTP and torrent servers), app E2E through WebDriver, extension E2E in Chromium, translation check, Windows cross-compile check | |

---|---|---|
| Shell | Custom title bar and window buttons, tray (minimize and close to tray), run on startup, single instance, window state, 6 themes, themable logo, settings, animated launch intro, first-run guide, portable mode, auto-update from GitHub Releases | Code signing for Windows and macOS installers |
| Engine | Probe, multi-connection download with dynamic splitting, slow connection replacement, pause/resume, queue, speed modes and limit, retry with backoff, offline wait and auto-resume, SQLite persistence, If-Range check with restart, Mark-of-the-Web, system/none/custom proxy (HTTP, SOCKS5) | Scheduler with time windows, FTP |
| Browser | MV3 extension (Chrome, Edge, Brave, Opera, Vivaldi, Firefox): catches downloads with cookies and referrer, holds them until you decide, context menus, download all or selected links, Alt+click bypass, popup; native messaging helper; auto-registration; named pipe | Link refresh, video detection (needs yt-dlp) |
| Organize | Type folders, SHA-256 check, drag and drop links, command line links, IDM .ef2 / text list import, clipboard watching, keep awake, action when everything finishes | Queues |
| Project | README with banner and screenshots, CI (fmt, clippy, tests on Windows/Linux/macOS, UI check), release workflow (installers for Windows, macOS arm64 and x64, Linux, plus extension zips), Dependabot | Code signing, store listings |

---

## 1. Name

**KlyxDown** (say "clicks down"). Checked on 2026-10-30:

| Check | Result |
|---|---|
| crates.io `klyxdown` | free (404) |
| npm `klyxdown` | free (404) |
| GitHub repos named `klyxdown` | 0 |
| `klyxdown.com` / `.app` (RDAP) | unregistered |
| Web search for "KlyxDown" | no product uses it |

Always use the full name. "Klyx" on its own is taken: an Android code editor (klyx-dev/klyx), an iOS game, a Google Play app and klyx.io; klyx.com is registered. "KDM" as a short form is also out: it is the KDE Display Manager (`kdm` is taken on Debian, crates.io and npm).

Backups, same checks, all free: `klyxdl`, `klyxget`, `klyxdm`.

Not checked yet: trademark registries (USPTO TESS, EUIPO eSearch). Do this before buying the domain.

Binary / package id: `klyxdown`. Bundle id: `net.spacie.klyxdown` (Tauri `identifier`, also the native messaging host name). The name lives in one place in code (`productName` in `src-tauri/tauri.conf.json`; the UI shows it from `src/lib/TitleBar.svelte` and `Logo.svelte`), so renaming later is cheap.

---

## 2. Product principles

1. **Fast by default, explained when asked.** Segmented, dynamically re-split downloads like IDM. The segment view shows what each connection is doing, so speed is never a mystery.
2. **One window, no dialogs stacked on dialogs.** Add, inspect and schedule from the same surface. Settings are searchable.
3. **Keyboard-first.** `Ctrl+N` add, `Ctrl+K` command bar, `Space` pause/resume, `/` filter, `Del` remove.
4. **Private and quiet.** No telemetry, no ads, no bundled offers. Crash reports are opt-in only.
5. **Cross-platform for real.** Windows, macOS, Linux on day one. IDM is Windows-only; that is our biggest opening.
6. **Safe downloads.** Mark-of-the-Web / quarantine flags on every file, optional antivirus scan, credentials in the OS keychain.

---

## 3. Feature parity matrix

Sources: freedownloadmanager.org (features, download pages), internetdownloadmanager.com (features2, news up to 6.43 b16, 2026-10-23).

Milestones: **M1** core engine, **M2** browser + media, **M3** organize + schedule, **M4** torrent + grabber, **M5** extras. "Out" = deliberately not doing.

### Core downloading

| Feature | IDM | FDM | KlyxDown |
|---|:-:|:-:|---|
| Multi-connection segmented download | yes | yes | M1 |
| Dynamic segmentation (split the largest remaining part when a connection frees up) | yes | partial | M1 |
| Reuse connections, no extra handshake per segment | yes | ? | M1 (HTTP keep-alive / HTTP/2 via reqwest) |
| Resume after crash, power loss, network drop | yes | yes | M1 (segment map checkpointed to SQLite every second) |
| Validate resume against server changes (ETag / Last-Modified, `If-Range`) | ? | ? | M1 |
| HTTP, HTTPS, HTTP/2, IPv6, TLS 1.3 | yes | yes | M1 |
| FTP / FTPS | yes | yes | M3 (`suppaftp`) |
| SFTP | no | no | M5 |
| MMS | yes | no | Out (dead protocol) |
| Multiple mirrors for one file | no | yes (3.9) | M3 (data model supports N sources from M1) |
| Metalink (`.meta4`) | no | yes (3.9) | M5 |
| Refresh an expired download link without losing progress | yes | no | M2 (extension re-captures the link from the parent page) |
| One-time temporary link handling | yes | ? | M2 (same mechanism) |
| Checksum verification (MD5, SHA-1, SHA-256) | no | no | M3 |
| Partial ZIP download (list and fetch one file from a remote ZIP) | no | yes (3.9) | M5 |
| Batch download with wildcards (`img[001-120].jpg`) | yes | yes | M3 |
| Import URL list from file / clipboard | yes | yes | M3 |

### Browser integration

| Feature | IDM | FDM | KlyxDown |
|---|:-:|:-:|---|
| Take over browser downloads | yes | yes | M2 |
| Chrome, Edge, Brave, Opera, Vivaldi, Firefox | yes | yes | M2 (one MV3 codebase) |
| Safari | yes | no | M5 (needs a Safari Web Extension wrapper, macOS only) |
| "Download with KlyxDown" context menu | yes | yes | M2 |
| Hold a key to skip takeover (Alt / Shift) | yes | ? | M2 |
| Minimum size before takeover | yes | ? | M2 |
| Pass cookies, referrer, user agent to the engine | yes | yes | M2 |
| Download all links / selected links on a page | yes | yes | M2 |
| Video "download this" button on web players | yes | yes | M2 (popup + page action, no injected overlay by default) |
| Catch downloads from any app (system hook) | yes | no | Out (OS-level hooking, antivirus flags it, fragile) |
| Built-in browser for sites the extension can't reach | no | no | M3 (see section 6) |

### Video and media

| Feature | IDM | FDM | KlyxDown |
|---|:-:|:-:|---|
| Grab video/audio from web players (HLS, DASH, progressive) | yes | yes (add-on) | M2 via `yt-dlp` sidecar |
| Merge separate audio + video streams | yes | yes | M2 via `ffmpeg` sidecar |
| Choose quality / format before download | yes | yes | M2 |
| TS to MP4 remux | yes | yes | M2 |
| Convert after download (presets) | yes | yes | M3 |
| Preview audio/video while downloading | no | yes | M3 (fetch head + tail segments first so MP4 `moov` is present) |

### BitTorrent

| Feature | IDM | FDM | KlyxDown |
|---|:-:|:-:|---|
| Torrent files + magnet links, DHT | no | yes | M4 (`librqbit`) |
| Pick files inside a torrent | no | yes | M4 |
| Per-torrent priority and bandwidth | no | yes | M4 |
| Stream / preview while downloading | no | yes | M4 (`librqbit` streams with piece prioritization) |
| Register as `magnet:` and `.torrent` handler | no | yes | M4 |

### Organize, schedule, traffic

| Feature | IDM | FDM | KlyxDown |
|---|:-:|:-:|---|
| Auto categories by type or site, custom folders | yes | yes | M3 |
| Multiple queues with their own concurrency | yes | yes | M3 |
| Scheduler: start/stop at times, weekdays | yes | yes | M3 |
| After done: shut down, sleep, hibernate, quit, run command | yes | yes | M3 |
| Keep the computer awake while downloading | no | no | M3 |
| Global + per-download speed limit | yes | yes | M1 |
| Traffic modes (low / medium / high / custom) | no | yes | M3 |
| Hourly quota (MB per hour) | yes | no | M3 |
| Pause on metered connection | no | no | M3 |
| Site grabber / spider with filters, depth, schedules | yes | yes (3.9) | M4 |
| Site explorer (browse a site or FTP tree, pick files) | no | yes (3.9) | M4 |
| Search and filter the list (`Ctrl+F`) | yes | yes | M1 |
| Columns: reorder, sort, "parent web page" | yes | partial | M1 |

### App, platform, extras

| Feature | IDM | FDM | KlyxDown |
|---|:-:|:-:|---|
| Windows / macOS / Linux | Windows | all | M1, all three |
| Light / dark / system theme | yes | yes | M1 |
| Interface languages incl. RTL (Arabic, Persian) | yes | 24 | M1 scaffold, community translations after |
| Clipboard monitoring (opt-in) | yes | yes | M3 |
| Drag links in, drag finished files out | yes | yes | M2 |
| Command line (`klyxdown add <url> --dir ...`) | yes | ? | M2 (forwarded to the running instance) |
| Antivirus scan after download | yes | no | M3 (Windows `IAttachmentExecute`, custom scanner command elsewhere) |
| Mark-of-the-Web / macOS quarantine | ? | ? | M1 |
| Proxy: system, HTTP(S), SOCKS5, per download | yes | yes | M3 |
| Proxy PAC files | yes | ? | M5 |
| Auth: Basic, Digest | yes | yes | M3 |
| Auth: NTLM, Kerberos | yes | no | Out for now (`reqwest` has no support; revisit on demand) |
| Portable mode | no | yes (3.9) | M3 |
| Remote control from phone | no | yes | M5 (local web UI, QR pairing, token auth) |
| Auto-update, signed | yes | yes | M1 |
| Import from IDM / FDM | n/a | n/a | M3 (IDM `.ef2` export, FDM URL lists) |
| Taskbar / dock progress | yes | yes | M1 |
| In-app file comments / ratings (FDM 3.9) | no | yes | Out (needs a server, moderation) |

---

## 4. Stack (versions checked 2026-10-30)

| Layer | Choice | Why |
|---|---|---|
| Shell | **Tauri 2.12.5** (stable; 3.0 is alpha, skip) | Small binaries, native webview, Rust core |
| Language | Rust 1.95 (edition 2024) | Installed locally |
| HTTP | `reqwest` 0.13 (rustls) | HTTP/2, proxies, streaming bodies |
| Async | `tokio` (Tauri's runtime) | |
| Storage | `rusqlite` 0.41 (bundled) | One file, WAL mode, no compile-time DB setup for contributors |
| FTP | `suppaftp` 12 | Async, TLS |
| BitTorrent | `librqbit` 9.1 | Pure Rust, magnets, DHT, streaming. Verify: V1-over-TCP only, sequential piece order |
| Media | `yt-dlp` + `ffmpeg` sidecars | Downloaded on first use with consent, checksum-verified, auto-updated |
| Rate limit | `governor` 0.10 | Token buckets; chain global + per-download |
| IPC types | `tauri-specta` 2.0.0-rc | Generates TS types for commands/events. Still RC: pin exact version |
| Logs | `tracing` + rolling files | |
| Errors | `thiserror` in crates, one serializable `AppError` at the IPC edge | |
| UI | **Svelte 5** + Vite 8 + TypeScript | Fine-grained updates suit 10 Hz progress ticks; tiny bundle |
| Motion | GSAP 3.15 (all plugins free) | Timelines for the few state transitions that matter |
| Icons | Phosphor | One family, one stroke weight |
| List | Virtualized rows (`@tanstack/svelte-virtual`) | 10k-row lists stay smooth |
| Extension | MV3 WebExtension + native messaging | No open localhost port |

---

## 5. Architecture

Hexagonal, but only where it pays: a trait exists when there are two or more real implementations (protocols, or real + test fake). No trait for one-implementation things.

```
klyxdown/
├─ crates/
│  ├─ engine/        # pure Rust, no Tauri. Domain + use cases.
│  │  ├─ model.rs        Download, Source, Segment, Queue, Schedule, events
│  │  ├─ segmenter.rs    dynamic split / steal algorithm (pure, heavily tested)
│  │  ├─ worker.rs       one connection: request range, stream, write
│  │  ├─ writer.rs       positional writes, preallocation, flush policy
│  │  ├─ limiter.rs      global + per-download token buckets
│  │  ├─ queue.rs        concurrency, priorities, scheduler rules
│  │  ├─ protocol.rs     trait Protocol { probe, open_range } + http, ftp impls
│  │  ├─ store.rs        SQLite persistence (schema + migrations via PRAGMA user_version)
│  │  └─ lib.rs          Engine handle: commands in, events out (tokio channels)
│  ├─ media/         # (M2) yt-dlp / ffmpeg sidecar driver
│  ├─ torrent/       # (M4) librqbit adapter
│  └─ bridge/        # (M2) native messaging host binary
├─ src-tauri/        # composition root: wires engine, commands, tray, plugins
├─ src/              # Svelte 5 app (Tauri standard layout: package.json at the root)
├─ extension/        # MV3 extension (Chrome, Edge, Firefox from one source)
└─ docs/             # this plan, ADRs (docs/adr/0001-*.md)
```

### Data flow

```
Browser ext ──native msg──> bridge ──local socket──┐
CLI args / deep link / magnet: ──single-instance──┤
UI (Svelte) ──invoke(command)─────────────────────┼──> Engine (tokio task)
                                                   │      ├─ Queue / Scheduler
UI <──Channel<ProgressBatch> (10 Hz)───────────────┤      ├─ Workers ──reqwest──> server
UI <──emit(DownloadEvent) on state changes─────────┘      ├─ Writer ──pwrite──> file.part
                                                          └─ Store ──WAL──> klyxdown.db
```

- **Commands** (`invoke`) for actions: add, pause, resume, remove, set limit, reorder.
- **One `tauri::ipc::Channel`** for progress, batched at 10 Hz for all active downloads in one message. Tauri docs recommend channels over events for streaming.
- **Events** only for discrete state changes (added, completed, failed), so other windows (tray popup) stay in sync.
- The engine owns all state. The UI is a projection; it never computes progress.

### Download engine details

1. **Probe.** `GET` with `Range: bytes=0-0` (some servers lie on `HEAD`). Read size, `Accept-Ranges`, `ETag`, `Last-Modified`, `Content-Disposition` filename, final URL after redirects.
2. **Single stream** if no range support or unknown size.
3. **Ramp up.** Start one connection at byte 0 immediately. Every time a connection becomes free (initial ramp, finished, failed), split the largest remaining segment at its midpoint if both halves stay above `min_split` (default 1 MiB). Max connections per download: default 8, cap 32.
4. **Safe splitting.** Each segment's `end` is an `AtomicU64`. The splitter lowers `end`; the worker always reads `end` before writing, so no byte is fetched twice past a small in-flight overlap that is simply discarded.
5. **Slow connection replacement.** If a connection runs under 25% of the median for 10 s, close it and let the split logic reassign its range.
6. **Writes.** Preallocate `name.ext.kdpart` (sparse where supported). Each worker buffers 256 KiB, then writes with positional I/O (`FileExt::seek_write` on Windows, `write_at` on Unix) on a blocking thread. No merge step at the end; rename on completion.
7. **Checkpoint.** Segment map saved to SQLite every second and on pause/exit. A crash loses at most one second of data.
8. **Resume.** Send `If-Range` with the stored ETag/Last-Modified. `200` instead of `206` means the file changed: ask the user, default to restart.
9. **Retry.** Exponential backoff with jitter, honor `Retry-After` on 429/503. After N failures on 403/410 or an HTML response where a file was expected: mark "link expired" and ask the extension to refresh it.
10. **Finish.** Verify size, optional checksum, apply Mark-of-the-Web (Windows `Zone.Identifier` ADS with `ReferrerUrl`/`HostUrl`) or `com.apple.quarantine`, run the AV scan if enabled, then move to the category folder.

### Persistence (SQLite, WAL)

Tables: `downloads`, `sources` (N per download, for mirrors), `segments`, `queues`, `schedules`, `categories`, `settings`, `history`. Credentials never go in SQLite: they go to the OS keychain (`keyring` crate).

### Security

- Tauri capabilities per window, least privilege. The main window gets only the app's own commands. The built-in browser window gets **no IPC at all**.
- Strict CSP, no remote scripts in the app UI.
- Sidecars are scoped by exact name in capabilities; no general shell execute.
- Native messaging host `net.spacie.klyxdown`; its manifest lists only our extension IDs (`allowed_origins` / `allowed_extensions`).
- Signed updates (`tauri-plugin-updater`), signed Windows builds, notarized macOS builds.
- `cargo deny` for licenses and advisories in CI.

---

## 6. Browser integration ("simple browser" covers two things)

### 6a. Extension (primary, M2)

One MV3 codebase for Chromium browsers and Firefox.

- `downloads.onCreated`: if takeover is on and no bypass key is held, cancel + erase the browser download and send `{url, finalUrl, referrer, filename, mime, size, cookies-for-that-URL, userAgent}` to the native host.
- Context menu: "Download with KlyxDown", "Download all links", "Download selected links".
- Media detection: observe `webRequest.onHeadersReceived` (non-blocking) for `video/*`, `audio/*`, `.m3u8`, `.mpd`. The toolbar badge shows the count; the popup lists them with quality options from `yt-dlp`.
- Link refresh: when the engine reports "link expired", the extension reopens the parent page in a background tab, catches the matching download (same name and size), returns the new URL and cancels it.
- Cookies: only for the download's own URL, only at handoff time. Never bulk-exported.
- Native messaging limits: host to browser max 1 MB per message, browser to host 64 MiB. Keep messages small.

The `bridge` binary is what the browser starts. It relays JSON to the running app over a local socket (named pipe on Windows, Unix socket elsewhere) and launches the app if it isn't running. The installer registers the host manifest for each browser (Windows registry keys under `HKCU\Software\<Browser>\NativeMessagingHosts`, JSON files on macOS/Linux).

### 6b. Built-in browser (M3)

For sites the extension can't help with, or users who don't want an extension: a second Tauri `WebviewWindow` with an address bar.

Verified in Tauri 2.12.5 source, all stable (no `unstable` flag):
- `WebviewWindowBuilder::on_download` intercepts downloads; return `false` and hand the URL to the engine.
- `WebviewWindow::cookies_for_url` passes the session to the engine so logged-in downloads work. **Windows caveat:** it deadlocks if called inside a synchronous handler; read cookies on a separate thread.
- `on_navigation` + `initialization_script` for media detection: the injected script reports found `<video>`/`<a>` URLs by navigating to `klyxdown-grab://found?...`, which `on_navigation` catches and cancels. Remote pages get zero IPC access this way.

Embedding the page inside the main window (`Window::add_child`) needs Tauri's `unstable` feature, so v1 uses a separate window.

---

## 7. Frontend

- Svelte 5 runes, one store per concern: `downloads` (fed by the progress channel), `settings`, `ui`. No global state library.
- State-based navigation; no router needed for ~6 views (list, details, add, scheduler, grabber, settings).
- Design tokens as CSS custom properties, light + dark + system, one accent, one radius scale.
- GSAP only for state changes: dialog open from its trigger, segment split, row completion. All wrapped in `gsap.matchMedia()` with a `prefers-reduced-motion` branch.
- Accessibility: every progress bar has `role="progressbar"` with throttled `aria-valuenow`, full keyboard paths, visible focus, RTL via logical CSS properties.
- i18n: JSON message files, ICU plurals, English source. Arabic/Persian RTL checked in CI with a pseudo-locale.

---

## 8. Testing

| Level | Tool | What |
|---|---|---|
| Unit | `cargo test` | Segmenter split/steal math, wildcard expansion, scheduler rules, filename sanitizing |
| Engine integration | Local `axum` test server | Range support on/off, throttling, random disconnects, changing ETag, 429 with `Retry-After`, expiring links |
| Property | `proptest` | "Bytes written == file size and every byte is covered exactly once" across random split/fail sequences |
| UI | Vitest + `@tauri-apps/api/mocks` | Components against mocked IPC |
| E2E | WebdriverIO + `tauri-driver` | Add, pause, resume, complete on Windows + Linux in CI |
| Extension | Playwright with the unpacked extension | Takeover, bypass key, context menu |

---

## 9. Repo hygiene and CI

- License: **GPL-3.0-or-later** recommended. Download managers attract ad-stuffed closed forks; GPL keeps forks open. (MIT/Apache-2.0 if you want maximum reuse instead. Your call.)
- `README`, `CONTRIBUTING`, `CODE_OF_CONDUCT`, `SECURITY.md`, issue/PR templates, ADRs in `docs/adr/`.
- Conventional commits + `release-please` for changelogs and versioning.
- GitHub Actions: `cargo fmt --check`, `clippy -D warnings`, `cargo test`, `cargo deny`, `biome check`, `vitest`, then `tauri-action` matrix (windows-latest, macos-latest arm64 + x64, ubuntu-22.04) on tags.
- Renovate for dependency updates, grouped weekly.
- Distribution: GitHub Releases, winget, Homebrew cask, Flathub, AUR. Extension on Chrome Web Store, Edge Add-ons, Firefox AMO.

---

## 10. Milestones (one developer, full time; double for evenings only)

| Milestone | Scope | Estimate |
|---|---|---|
| **M0** Skeleton | Workspace, Tauri app, chosen UI style as tokens, CI green, empty list | 3-4 days |
| **M1** Core engine | HTTP(S) segmented + dynamic split, resume, limiter, SQLite, list UI, add dialog, tray, updater, MOTW | 3-4 weeks |
| **M2** Browser + media | Extension + bridge, link refresh, yt-dlp/ffmpeg sidecars, CLI, drag and drop | 3 weeks |
| **M3** Organize + schedule | Categories, queues, scheduler, traffic modes, FTP, mirrors, proxy, checksums, AV scan, built-in browser, import from IDM/FDM, portable | 4 weeks |
| **M4** Torrent + grabber | librqbit, magnet handler, streaming, site grabber/explorer | 3 weeks |
| **M5** Extras | Remote control, Metalink, partial ZIP, SFTP, PAC, Safari | ongoing |

First public beta after M2 (about 7 weeks in): that already covers what most IDM users actually use.

---

## 11. Open decisions

1. License: GPL-3.0-or-later (recommended) or MIT/Apache-2.0.
