// End-to-end: the real app (built with its UI embedded), driven through WebDriver
// (tauri-driver + WebKitWebDriver), downloading from a local server.
//
//   npm run build && cargo build -p klyxdown --features tauri/custom-protocol
//   dbus-run-session -- xvfb-run -a node --test e2e/app.test.mjs        (Linux)
//
// Each run gets its own settings and data folders, so it never touches a real install.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { after, before, test } from 'node:test';
import { remote } from 'webdriverio';

const APP = resolve(process.env.KLYXDOWN_APP ?? 'target/debug/klyxdown');
const SIZE = 6 * 1024 * 1024 + 321;
const BODY = Buffer.alloc(SIZE);
for (let i = 0; i < SIZE; i++) BODY[i] = Math.imul(i, 2654435761) >>> 13;
const TOKEN = 'e2e-0123456789abcdef0123456789abcdef';

let server, base, driver, browser, home, downloads, remotePort;

const freePort = () =>
  new Promise((done) => {
    const s = net.createServer().listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => done(port));
    });
  });

/** Files with byte ranges; /slow/ ones trickle so they can be paused. */
function serve() {
  return http.createServer((req, res) => {
    const slow = req.url.startsWith('/slow/');
    const m = /bytes=(\d+)-(\d*)/.exec(req.headers.range ?? '');
    const start = m ? Number(m[1]) : 0;
    const end = m && m[2] ? Math.min(Number(m[2]), SIZE - 1) : SIZE - 1;
    res.writeHead(m ? 206 : 200, {
      'content-length': end - start + 1,
      'accept-ranges': 'bytes',
      'content-type': 'application/octet-stream',
      ...(m ? { 'content-range': `bytes ${start}-${end}/${SIZE}` } : {}),
    });
    if (!slow) return res.end(BODY.subarray(start, end + 1));
    let at = start;
    const timer = setInterval(() => {
      if (at > end || res.destroyed) return clearInterval(timer), res.end();
      const next = Math.min(at + 32 * 1024, end + 1);
      res.write(BODY.subarray(at, next));
      at = next;
    }, 100);
    res.on('close', () => clearInterval(timer));
  });
}

async function until(what, check, ms = 30000) {
  const deadline = Date.now() + ms;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > deadline) throw new Error(`timed out: ${what}`);
    await new Promise((r) => setTimeout(r, 200));
  }
}

const done = (name) => {
  const path = join(downloads, name);
  return existsSync(path) && statSync(path).size === SIZE && !existsSync(`${path}.kdpart`);
};

async function addLink(url) {
  const box = await browser.$('input[aria-label="Paste a link or magnet"]');
  await box.setValue(url);
  await browser.keys('Enter');
  const start = await browser.$('button=Start download');
  await start.waitForEnabled({ timeout: 20000 });
  await start.click();
}

before(async () => {
  server = serve();
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${server.address().port}`;
  home = mkdtempSync(join(tmpdir(), 'klyxdown-e2e-'));
  downloads = join(home, 'Downloads');
  mkdirSync(downloads);
  remotePort = await freePort();
  const config = join(home, 'config', 'net.spacie.klyxdown');
  mkdirSync(config, { recursive: true });
  writeFileSync(
    join(config, 'settings.json'),
    JSON.stringify({
      onboarded: true,
      showIntro: false,
      downloadDir: downloads,
      sortByType: false,
      autoUpdate: false,
      handleMagnets: false,
      closeToTray: false,
      minimizeToTray: false,
      scanAfter: false,
      remote: true,
      remotePort,
      remoteToken: TOKEN,
    }),
  );
  const env = { ...process.env, XDG_CONFIG_HOME: join(home, 'config'), XDG_DATA_HOME: join(home, 'data'), XDG_CACHE_HOME: join(home, 'cache'), HOME: home };
  driver = spawn(process.env.TAURI_DRIVER ?? 'tauri-driver', [], { env, stdio: ['ignore', 'inherit', 'inherit'] });
  await until('tauri-driver', () => new Promise((r) => net.connect(4444, '127.0.0.1').on('connect', () => r(true)).on('error', () => r(false))), 15000);
  browser = await remote({
    hostname: '127.0.0.1',
    port: 4444,
    logLevel: 'warn',
    capabilities: { 'tauri:options': { application: APP } },
  });
  await (await browser.$('.shell')).waitForExist({ timeout: 30000 });
});

after(async () => {
  await browser?.deleteSession().catch(() => {});
  driver?.kill();
  server?.close();
});

test('a pasted link downloads byte for byte', async () => {
  await addLink(`${base}/files/first.bin`);
  await until('first.bin on disk', () => done('first.bin'));
  assert.ok(readFileSync(join(downloads, 'first.bin')).equals(BODY));
  // It shows up as finished in the shelf.
  await (await browser.$('button*=Finished')).click();
  await (await browser.$('b=first.bin')).waitForExist({ timeout: 10000 });
});

test('pause keeps the progress and resume finishes it', async () => {
  await addLink(`${base}/slow/second.bin`);
  const pause = await browser.$('button[aria-label="Pause"]');
  await pause.waitForExist({ timeout: 15000 });
  await pause.click();
  await (await browser.$('button*=Paused')).click();
  const resume = await browser.$('button[aria-label="Start"]');
  await resume.waitForExist({ timeout: 10000 });
  assert.ok(existsSync(join(downloads, 'second.bin.kdpart')), 'the partial file stays');
  await resume.click();
  await until('second.bin on disk', () => done('second.bin'), 120000);
  assert.ok(readFileSync(join(downloads, 'second.bin')).equals(BODY));
});

test('search narrows the lists', async () => {
  const search = await browser.$('input[aria-label="Search downloads"]');
  await search.setValue('first');
  await (await browser.$('button*=Latest')).click();
  let names = [];
  await until('only first.bin listed', async () => {
    names = await browser.execute(() => [...document.querySelectorAll('.shelf .row b')].map((b) => b.textContent));
    return names.length === 1 && names[0] === 'first.bin';
  }).catch((e) => {
    throw new Error(`${e.message}; listed: ${JSON.stringify(names)}`);
  });
  await search.clearValue();
});

test('a night queue can be added', async () => {
  await (await browser.$('button[aria-label="Queues"]')).click();
  await (await browser.$('button*=Add a night queue')).click();
  const name = await browser.$('input[aria-label="Queue name"]');
  await name.waitForExist({ timeout: 10000 });
  assert.equal(await name.getValue(), 'Night');
  await browser.keys('Escape');
});

test('the phone remote needs its token', async () => {
  const api = `http://127.0.0.1:${remotePort}/api/list`;
  await until('remote server', () => fetch(api).then(() => true, () => false));
  assert.equal((await fetch(api)).status, 401);
  assert.equal((await fetch(api, { headers: { 'x-klyxdown-token': 'wrong' } })).status, 401);
  const list = await (await fetch(api, { headers: { 'x-klyxdown-token': TOKEN } })).json();
  assert.ok(list.some((d) => d.name === 'first.bin' && d.state === 'done'), JSON.stringify(list));
  const page = await fetch(`http://127.0.0.1:${remotePort}/?t=${TOKEN}`);
  assert.equal(page.status, 200);
  assert.match(await page.text(), /<title>KlyxDown<\/title>/);
  assert.ok(readdirSync(downloads).includes('first.bin'));
});

test('removing a paused and a running download leaves the app running', async () => {
  const gone = (name) => until(`${name} removed`, async () => !(await browser.execute((n) => document.body.textContent.includes(n), name)));
  await addLink(`${base}/slow/third.bin`);
  const pause = await browser.$('button[aria-label="Pause"]');
  await pause.waitForExist({ timeout: 15000 });
  await pause.click();
  await (await browser.$('button*=Paused')).click();
  const x = await browser.$('.shelf .row[data-state="paused"] button[aria-label="Remove from list"]');
  await x.waitForExist({ timeout: 10000 });
  await x.click();
  await gone('third.bin');

  await addLink(`${base}/slow/fourth.bin`);
  const row = await browser.$('.ch[data-state="downloading"]');
  await row.waitForExist({ timeout: 15000 });
  await row.click();
  await (await browser.$('.ch.open button*=Remove from list')).click();
  await gone('fourth.bin');
  assert.equal(await browser.execute(() => 1 + 1), 2, 'the app still answers');
});
