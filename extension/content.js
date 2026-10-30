// Alt+click on a link lets the browser download it itself; KlyxDown stays out of the way.
addEventListener(
  'click',
  (e) => {
    if (e.altKey && e.target instanceof Element && e.target.closest('a[href]')) {
      chrome.runtime.sendMessage({ type: 'bypass' }).catch(() => {});
    }
  },
  true,
);
