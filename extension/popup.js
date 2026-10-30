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

const reply = await chrome.runtime.sendMessage({ type: 'ping' });
if (reply?.ok) {
  status.textContent = `Connected to KlyxDown ${reply.version ?? ''}`.trim();
  status.className = 'status ok';
} else {
  status.textContent = 'KlyxDown is not installed or could not start. Downloads stay in the browser.';
  status.className = 'status bad';
}
