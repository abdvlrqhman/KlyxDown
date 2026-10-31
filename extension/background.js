// KlyxDown browser extension: catches downloads and hands them to the app through the
// native messaging host (crates/bridge). Works in Chrome, Edge, Brave, Opera, Vivaldi and Firefox.

const HOST = 'net.spacie.klyxdown';
const DEFAULTS = { enabled: true, minSizeMb: 0 };

// Set by content.js when the user Alt+clicks a link: let the browser handle that one.
let bypassUntil = 0;

const settings = () => chrome.storage.local.get(DEFAULTS);

async function send(message) {
  try {
    return await chrome.runtime.sendNativeMessage(HOST, message);
  } catch (e) {
    return { ok: false, error: String(e?.message ?? e) };
  }
}

function browserName() {
  const ua = navigator.userAgent;
  if (ua.includes('Firefox/')) return 'Firefox';
  if (ua.includes('Edg/')) return 'Edge';
  if (ua.includes('OPR/')) return 'Opera';
  if (ua.includes('Vivaldi')) return 'Vivaldi';
  if (navigator.brave) return 'Brave';
  return 'Chrome';
}

// Only the cookies of the download's own site, only when handing it over.
async function cookieHeader(url) {
  try {
    const list = await chrome.cookies.getAll({ url });
    return list.map((c) => `${c.name}=${c.value}`).join('; ') || null;
  } catch {
    return null;
  }
}

// Opening an image in a new tab is viewing it, even when the server sends it as a download.
// Images stay with the browser (like IDM's default file types); "Download with KlyxDown" still works.
const IMAGE = /\.(jpe?g|png|gif|webp|avif|svg|bmp|ico|heic|jfif)$/i;
function isImage(item, url) {
  if (item.mime?.startsWith('image/')) return true;
  try {
    return IMAGE.test(baseName(item.filename) || new URL(url).pathname);
  } catch {
    return false;
  }
}

function baseName(path) {
  return path ? path.split(/[\\/]/).pop() || null : null;
}

function report(reply) {
  if (reply?.ok || reply?.declined) {
    chrome.action.setBadgeText({ text: '' });
    chrome.action.setTitle({ title: 'KlyxDown' });
  } else {
    chrome.action.setBadgeBackgroundColor({ color: '#F2A28E' });
    chrome.action.setBadgeText({ text: '!' });
    chrome.action.setTitle({ title: `KlyxDown is not reachable: ${reply?.error ?? 'unknown error'}` });
  }
  return reply;
}

chrome.downloads.onCreated.addListener(async (item) => {
  const url = item.finalUrl || item.url;
  if (!/^https?:/i.test(url) || Date.now() < bypassUntil || isImage(item, url)) return;
  const { enabled, minSizeMb } = await settings();
  if (!enabled) return;
  if (minSizeMb > 0 && item.totalBytes > 0 && item.totalBytes < minSizeMb * 1e6) return;

  // Hold the browser's copy until the app confirms, so nothing is lost if the app is missing.
  await chrome.downloads.pause(item.id).catch(() => {});
  const [current] = await chrome.downloads.search({ id: item.id });
  if (current?.state === 'complete') return;

  const reply = report(
    await send({
      type: 'download',
      url,
      filename: baseName(item.filename),
      size: item.totalBytes > 0 ? item.totalBytes : null,
      referrer: item.referrer || null,
      cookies: await cookieHeader(url),
      userAgent: navigator.userAgent,
      browser: browserName(),
    }),
  );
  if (reply.ok) {
    await chrome.downloads.cancel(item.id).catch(() => {});
    await chrome.downloads.erase({ id: item.id }).catch(() => {});
  } else {
    // App unreachable, or the user chose to let the browser keep it.
    await chrome.downloads.resume(item.id).catch(() => {});
  }
});

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: 'kd-link', title: 'Download with KlyxDown', contexts: ['link'] });
    chrome.contextMenus.create({ id: 'kd-media', title: 'Download with KlyxDown', contexts: ['video', 'audio', 'image'] });
    chrome.contextMenus.create({ id: 'kd-selection', title: 'Download selected links with KlyxDown', contexts: ['selection'] });
    chrome.contextMenus.create({ id: 'kd-page', title: 'Download all links with KlyxDown', contexts: ['page'] });
  });
});

// Runs inside the page: every link (or only the selected ones) plus media sources.
function collectLinks(selectionOnly) {
  const anchors = [...document.querySelectorAll('a[href]')];
  const sel = getSelection();
  const picked = selectionOnly && sel && sel.rangeCount ? anchors.filter((a) => sel.containsNode(a, true)) : anchors;
  const media = selectionOnly ? [] : [...document.querySelectorAll('video[src], audio[src], source[src]')].map((e) => e.src);
  return [...new Set([...picked.map((a) => a.href), ...media])].filter((u) => /^https?:/i.test(u));
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const referrer = info.pageUrl || tab?.url || null;
  if (info.menuItemId === 'kd-link' || info.menuItemId === 'kd-media') {
    const url = info.linkUrl || info.srcUrl;
    if (!url || !/^https?:/i.test(url)) return;
    report(await send({ type: 'download', url, referrer, cookies: await cookieHeader(url), userAgent: navigator.userAgent, browser: browserName() }));
    return;
  }
  if (!tab?.id) return;
  const [result] = await chrome.scripting.executeScript({
    target: { tabId: tab.id, frameIds: [info.frameId ?? 0] },
    func: collectLinks,
    args: [info.menuItemId === 'kd-selection'],
  });
  // No cookies here: the links can point at other sites, which must not get this site's cookies.
  report(await send({ type: 'links', urls: result?.result ?? [], title: tab.title ?? null, referrer, userAgent: navigator.userAgent, browser: browserName() }));
});

chrome.runtime.onMessage.addListener((msg, _sender, respond) => {
  if (msg?.type === 'bypass') {
    bypassUntil = Date.now() + 4000;
    return false;
  }
  if (msg?.type === 'ping' || msg?.type === 'show') {
    send({ type: msg.type }).then((reply) => respond(report(reply)));
    return true; // respond asynchronously
  }
  return false;
});
