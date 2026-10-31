// End-to-end for the browser extension: Chromium (Playwright) with the unpacked extension and
// a stand-in native host that records every message and says yes.
//
//   npx playwright install chromium && node --test e2e/extension.test.mjs
import assert from 'node:assert/strict';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { after, before, test } from 'node:test';
import { chromium } from '@playwright/test';

const EXTENSION = resolve('extension');
const EXTENSION_ID = 'aohhcdaipkgppefknfnindgdacepogeb';
let server, base, context, worker, dir, log;

/** What the stand-in host received so far. */
const received = () => (existsSync(log) ? readFileSync(log, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse) : []);

async function until(what, check, ms = 15000) {
  const deadline = Date.now() + ms;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > deadline) throw new Error(`timed out: ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

before(async () => {
  const clip = Buffer.alloc(700 * 1024);
  server = http.createServer((req, res) => {
    if (req.url === '/') {
      res.writeHead(200, { 'content-type': 'text/html', 'set-cookie': 'session=abc123; Path=/' });
      return res.end(`<a id="zip" href="/file.zip">zip</a> <a id="pic" href="/pic.png">pic</a> <a id="alt" href="/other.zip">other</a>`);
    }
    if (req.url === '/video') {
      res.writeHead(200, { 'content-type': 'text/html' });
      return res.end(`<video id="v" src="/clip.mp4" muted autoplay></video>`);
    }
    if (req.url === '/clip.mp4') {
      res.writeHead(200, { 'content-type': 'video/mp4', 'content-length': clip.length });
      return res.end(clip);
    }
    // Real downloads take a while; a finished one is left to the browser.
    const type = req.url.endsWith('.png') ? 'image/png' : 'application/zip';
    const body = Buffer.alloc(512 * 1024);
    res.writeHead(200, { 'content-type': type, 'content-disposition': 'attachment', 'content-length': body.length });
    res.write(body.subarray(0, 1024));
    const timer = setTimeout(() => res.end(body.subarray(1024)), 3000);
    res.on('close', () => clearTimeout(timer));
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${server.address().port}`;

  dir = mkdtempSync(join(tmpdir(), 'klyxdown-ext-'));
  log = join(dir, 'host.log');
  const host = join(dir, 'host.mjs');
  // Native messaging: 4-byte little-endian length, then JSON, both ways.
  writeFileSync(
    host,
    `#!/usr/bin/env node
import { appendFileSync } from 'node:fs';
let buf = Buffer.alloc(0);
process.stdin.on('data', (d) => {
  buf = Buffer.concat([buf, d]);
  while (buf.length >= 4 && buf.length >= 4 + buf.readUInt32LE(0)) {
    const len = buf.readUInt32LE(0);
    const msg = buf.subarray(4, 4 + len).toString();
    buf = buf.subarray(4 + len);
    appendFileSync(${JSON.stringify(log)}, msg + '\\n');
    const out = Buffer.from(JSON.stringify({ ok: true, version: 'test' }));
    const head = Buffer.alloc(4);
    head.writeUInt32LE(out.length);
    process.stdout.write(Buffer.concat([head, out]));
  }
});
`,
  );
  chmodSync(host, 0o755);
  // Chromium also reads native host manifests from the profile folder.
  const profile = join(dir, 'profile');
  mkdirSync(join(profile, 'NativeMessagingHosts'), { recursive: true });
  writeFileSync(
    join(profile, 'NativeMessagingHosts', 'net.spacie.klyxdown.json'),
    JSON.stringify({ name: 'net.spacie.klyxdown', description: 'test', path: host, type: 'stdio', allowed_origins: [`chrome-extension://${EXTENSION_ID}/`] }),
  );
  context = await chromium.launchPersistentContext(profile, {
    channel: 'chromium',
    headless: true,
    acceptDownloads: true,
    args: [`--disable-extensions-except=${EXTENSION}`, `--load-extension=${EXTENSION}`],
  });
  worker = context.serviceWorkers()[0] ?? (await context.waitForEvent('serviceworker'));
  assert.ok(worker.url().startsWith(`chrome-extension://${EXTENSION_ID}/`), worker.url());
});

after(async () => {
  await context?.close();
  server?.close();
});

test('downloads go to KlyxDown with the referrer and the site cookie', async () => {
  const page = await context.newPage();
  await page.goto(`${base}/`);
  await page.click('#zip');
  const msg = await until('download message', () => received().find((m) => m.type === 'download' && m.url.endsWith('/file.zip')));
  assert.equal(msg.referrer, `${base}/`);
  assert.equal(msg.cookies, 'session=abc123');
  assert.equal(msg.browser, 'Chrome');
  // The browser's own copy is cancelled and erased once KlyxDown takes it.
  await until('browser copy gone', async () => (await worker.evaluate(() => chrome.downloads.search({}))).every((d) => !d.url.endsWith('/file.zip')));
  await page.close();
});

test('images and Alt-clicks stay with the browser', async () => {
  const page = await context.newPage();
  await page.goto(`${base}/`);
  await page.click('#pic');
  await page.click('#alt', { modifiers: ['Alt'] });
  await new Promise((r) => setTimeout(r, 1500));
  const urls = received().map((m) => m.url ?? '');
  assert.ok(!urls.some((u) => u.endsWith('/pic.png')), 'image was taken over');
  assert.ok(!urls.some((u) => u.endsWith('/other.zip')), 'Alt-click was taken over');
  await page.close();
});

test('video a page plays is found and handed over with its page', async () => {
  const page = await context.newPage();
  await page.goto(`${base}/video`);
  const found = await until('media found', async () => {
    const all = await worker.evaluate(() => chrome.storage.session.get(null));
    return Object.values(all).flat().find((m) => m.url?.endsWith('/clip.mp4'));
  });
  assert.equal(found.kind, 'video');
  // What the popup's "This file" button sends, for the active tab.
  await page.bringToFront();
  const reply = await worker.evaluate(async (url) => {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    return sendMedia(tab, url);
  }, found.url);
  assert.equal(reply.ok, true);
  const msg = await until('media message', () => received().find((m) => m.type === 'media'));
  assert.equal(msg.page, `${base}/video`);
  assert.equal(msg.url, found.url);
  await page.close();
});
