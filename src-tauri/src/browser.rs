//! The built-in browser, for sites the extension can't reach or people without it.
//!
//! A separate window showing the web page itself. It has no access to the app (no IPC:
//! the capabilities only cover the main window). A small toolbar is drawn into each page
//! by an init script; its one privileged action, "get the video", navigates to
//! `klyxdown-grab://media`, which `on_navigation` catches and cancels. Downloads the page
//! starts are taken over with the page's cookies, so signed-in downloads work.

use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

use crate::bridge::{self, Captured, CapturedMedia};

const LABEL: &str = "browser";
const HOME: &str = "https://duckduckgo.com/";
const GRAB: &str = "klyxdown-grab";

/// Back, forward, reload, address, and "get the video", in a closed shadow root so pages
/// can't style it. Top frame only.
const TOOLBAR: &str = r#"
(() => {
  if (window.top !== window || window.__klyxdownBar) return;
  window.__klyxdownBar = true;
  const mount = () => {
    const host = document.createElement('klyxdown-bar');
    const root = host.attachShadow({ mode: 'closed' });
    root.innerHTML = `<style>
      :host { all: initial; position: fixed; z-index: 2147483647; right: 12px; bottom: 12px; }
      .bar { display: flex; gap: 4px; align-items: center; padding: 5px; border-radius: 10px;
        background: #121b2bee; box-shadow: 0 8px 30px #0008; font: 13px system-ui, sans-serif; color: #e4eaf3; }
      button { border: 0; border-radius: 6px; padding: 6px 9px; background: #1a2538; color: inherit; font: inherit; cursor: pointer; }
      button:hover { background: #202d43; }
      .go { background: #8dbaf2; color: #0a1424; font-weight: 600; }
      input { width: 260px; border: 0; border-radius: 6px; padding: 6px 8px; background: #08101b; color: inherit; font: inherit; outline: none; }
    </style>
    <div class="bar" role="toolbar" aria-label="KlyxDown browser">
      <button data-a="back" title="Back">&#8592;</button>
      <button data-a="forward" title="Forward">&#8594;</button>
      <button data-a="reload" title="Reload">&#8635;</button>
      <form><input aria-label="Address" spellcheck="false"></form>
      <button class="go" data-a="video" title="Download the video on this page with KlyxDown">Get the video</button>
    </div>`;
    const input = root.querySelector('input');
    input.value = location.href;
    root.querySelector('form').addEventListener('submit', (e) => {
      e.preventDefault();
      let to = input.value.trim();
      if (!/^https?:\/\//i.test(to)) to = to.includes('.') && !to.includes(' ') ? 'https://' + to : 'https://duckduckgo.com/?q=' + encodeURIComponent(to);
      location.href = to;
    });
    root.addEventListener('click', (e) => {
      const a = e.target.closest('button')?.dataset.a;
      if (a === 'back') history.back();
      else if (a === 'forward') history.forward();
      else if (a === 'reload') location.reload();
      else if (a === 'video') location.href = 'klyxdown-grab://media?page=' + encodeURIComponent(location.href);
    });
    document.documentElement.append(host);
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount);
  else mount();
})();
"#;

/// Opens the browser window (or brings it back), at `url` when given.
pub fn open(app: &AppHandle, url: Option<&str>) -> tauri::Result<()> {
    let target = url
        .and_then(|u| Url::parse(u.trim()).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https"));
    if let Some(window) = app.get_webview_window(LABEL) {
        if let Some(u) = target {
            window.navigate(u)?;
        }
        window.show()?;
        return window.set_focus();
    }
    let start = target.unwrap_or_else(|| Url::parse(HOME).expect("valid home page"));
    let (nav_app, dl_app, popup_app) = (app.clone(), app.clone(), app.clone());
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(start))
        .title("KlyxDown Browser")
        .inner_size(1100.0, 760.0)
        .min_inner_size(500.0, 400.0)
        .initialization_script(TOOLBAR)
        .on_navigation(move |url| {
            if url.scheme() != GRAB {
                return true;
            }
            let page = url
                .query_pairs()
                .find(|(k, _)| k == "page")
                .map(|(_, v)| v.into_owned())
                .filter(|p| p.starts_with("https://") || p.starts_with("http://"));
            if let Some(page) = page {
                let app = nav_app.clone();
                // Cookies are read off the webview's own thread (it deadlocks on Windows).
                std::thread::spawn(move || hand_over_page(&app, page));
            }
            false
        })
        .on_download(move |webview, event| {
            if let DownloadEvent::Requested { url, .. } = event {
                let app = dl_app.clone();
                let referrer = webview.url().ok().map(|u| u.to_string());
                std::thread::spawn(move || hand_over_download(&app, url, referrer));
                // KlyxDown downloads it instead of the webview.
                return false;
            }
            true
        })
        // Pop-ups and "open in new tab" stay in this one window.
        .on_new_window(move |url, _| {
            let app = popup_app.clone();
            let _ = popup_app.run_on_main_thread(move || {
                if let Some(w) = app.get_webview_window(LABEL) {
                    let _ = w.navigate(url);
                }
            });
            NewWindowResponse::Deny
        })
        .build()?;
    Ok(())
}

fn cookie_header(app: &AppHandle, url: &Url) -> Option<String> {
    let window = app.get_webview_window(LABEL)?;
    let cookies = window.cookies_for_url(url.clone()).ok()?;
    let header = cookies
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>()
        .join("; ");
    (!header.is_empty()).then_some(header)
}

fn hand_over_download(app: &AppHandle, url: Url, referrer: Option<String>) {
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }
    let captured = Captured {
        id: 0,
        url: url.to_string(),
        name: None,
        size: None,
        headers: bridge::request_headers(referrer, cookie_header(app, &url), None),
        browser: "KlyxDown Browser".into(),
    };
    bridge::deliver_capture(app, captured);
}

fn hand_over_page(app: &AppHandle, page: String) {
    let Ok(url) = Url::parse(&page) else { return };
    let media = CapturedMedia {
        headers: bridge::request_headers(Some(page.clone()), cookie_header(app, &url), None),
        page,
        url: None,
        title: None,
        browser: "KlyxDown Browser".into(),
    };
    bridge::deliver(app, "capture-media", media, |p, m| p.media = Some(m));
}
