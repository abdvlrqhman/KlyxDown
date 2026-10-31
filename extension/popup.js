const $ = (s) => document.querySelector(s);
const status = $('#status');

const stored = await chrome.storage.local.get({ enabled: true, minSizeMb: 0 });
$('#enabled').checked = stored.enabled;
$('#minSize').value = String(stored.minSizeMb);

$('#enabled').addEventListener('change', (e) => chrome.storage.local.set({ enabled: e.target.checked }));
$('#minSize').addEventListener('change', (e) => chrome.storage.local.set({ minSizeMb: Number(e.target.value) }));
$('#open').addEventListener('click', async () => {
  await chrome.runtime.sendMessage({ type: 'show' });
  window.close();
});

// Media the page played: one button for the page (yt-dlp picks the quality), and each
// plain file on its own.
const { list, page } = await chrome.runtime.sendMessage({ type: 'found' });
if (list?.length && /^https?:/i.test(page ?? '')) {
  $('#media').hidden = false;
  const names = { video: 'Video file', audio: 'Audio file', stream: 'Stream' };
  for (const item of list.slice(-6).reverse()) {
    const li = document.createElement('li');
    const label = document.createElement('span');
    const size = item.size ? ` · ${(item.size / 1e6).toFixed(1)} MB` : '';
    label.textContent = `${names[item.kind] ?? 'Media'}${size}`;
    label.title = item.url;
    li.append(label);
    if (item.kind !== 'stream') {
      const get = document.createElement('button');
      get.type = 'button';
      get.textContent = 'This file';
      get.addEventListener('click', async () => {
        await chrome.runtime.sendMessage({ type: 'media', url: item.url });
        window.close();
      });
      li.append(get);
    }
    $('#found').append(li);
  }
}
$('#grab').addEventListener('click', async () => {
  await chrome.runtime.sendMessage({ type: 'media' });
  window.close();
});

const reply = await chrome.runtime.sendMessage({ type: 'ping' });
if (reply?.ok) {
  status.textContent = `Connected to KlyxDown ${reply.version ?? ''}`.trim();
  status.className = 'status ok';
} else {
  status.textContent = 'KlyxDown is not installed or could not start. Downloads stay in the browser.';
  status.className = 'status bad';
}
