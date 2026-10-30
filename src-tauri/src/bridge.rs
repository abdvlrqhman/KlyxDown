//! Link to the browser extension.
//!
//! The extension talks to the native messaging host (`crates/bridge`), which
//! relays each message here over a local socket: a named pipe on Windows, a Unix
//! socket elsewhere. Nothing listens on the network. This module also registers
//! that host with every installed browser at startup.

use std::{
    collections::HashMap,
    fs, io,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use klyxdown_engine::AddRequest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::oneshot,
};

use crate::{AppState, tray};

pub const HOST_NAME: &str = "net.spacie.klyxdown";
/// Fixed by the `key` in extension/manifest.json.
const CHROME_EXTENSION_ID: &str = "aohhcdaipkgppefknfnindgdacepogeb";
const FIREFOX_EXTENSION_ID: &str = "klyxdown@spacie.net";

const MAX_FRAME: usize = 1 << 20;
const MAX_URL: usize = 8 * 1024;
const MAX_LINKS: usize = 2000;
const MAX_HEADER: usize = 16 * 1024;
/// How long the browser keeps its copy paused while the user decides. Chrome stops an
/// extension service worker after 5 minutes of work on one event, so stay well under that.
const DECISION_TIMEOUT: Duration = Duration::from_secs(170);

// Keep in sync with crates/bridge/src/main.rs.
#[cfg(windows)]
const PIPE: &str = r"\\.\pipe\net.spacie.klyxdown";

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Incoming {
    Ping,
    Show,
    Download {
        url: String,
        filename: Option<String>,
        size: Option<u64>,
        referrer: Option<String>,
        cookies: Option<String>,
        user_agent: Option<String>,
        browser: Option<String>,
    },
    Links {
        urls: Vec<String>,
        title: Option<String>,
        referrer: Option<String>,
        cookies: Option<String>,
        user_agent: Option<String>,
        browser: Option<String>,
    },
}

/// A download handed over by the browser, shown to the user before it starts.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Captured {
    /// Non-zero when the browser is waiting for the user's decision (see [`Decisions`]).
    pub id: u64,
    pub url: String,
    pub name: Option<String>,
    pub size: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub browser: String,
}

/// Every link on a page (or in a selection), for the user to pick from.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedLinks {
    pub urls: Vec<String>,
    pub title: Option<String>,
    pub headers: Vec<(String, String)>,
    pub browser: String,
}

/// Hand-offs that arrived while the UI was still loading; `commands::ui_ready` delivers them.
#[derive(Default)]
pub struct Pending {
    pub capture: Option<Captured>,
    pub links: Option<CapturedLinks>,
}

/// Browser downloads waiting for the user to take them or hand them back.
/// Until then the browser keeps its copy paused, so nothing is lost either way.
#[derive(Default)]
pub struct Decisions {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
}

impl Decisions {
    fn register(&self) -> (u64, oneshot::Receiver<bool>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        (id, rx)
    }

    /// `taken`: KlyxDown downloads it. Otherwise the browser resumes its own copy.
    pub fn answer(&self, id: u64, taken: bool) {
        if let Some(tx) = self
            .waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        {
            let _ = tx.send(taken);
        }
    }
}

#[derive(Serialize)]
struct Reply {
    ok: bool,
    /// The user chose to let the browser keep it; not an error.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    declined: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'static str>,
}

impl Reply {
    fn ok() -> Self {
        Self {
            ok: true,
            declined: false,
            error: None,
            version: Some(env!("CARGO_PKG_VERSION")),
        }
    }
    fn err(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            declined: false,
            error: Some(text.into()),
            version: None,
        }
    }
    fn declined() -> Self {
        Self {
            ok: false,
            declined: true,
            error: None,
            version: None,
        }
    }
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = serve(app).await {
            tracing::warn!("browser link stopped: {e}");
        }
    });
}

#[cfg(windows)]
async fn serve(app: AppHandle) -> io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    // first_pipe_instance: never attach to a pipe some other process created.
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(PIPE)?;
    loop {
        server.connect().await?;
        let next = ServerOptions::new()
            .reject_remote_clients(true)
            .create(PIPE)?;
        let conn = std::mem::replace(&mut server, next);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = handle(conn, &app).await {
                tracing::debug!("browser message failed: {e}");
            }
        });
    }
}

#[cfg(unix)]
async fn serve(app: AppHandle) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = socket_path();
    let _ = fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    loop {
        let (conn, _) = listener.accept().await?;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = handle(conn, &app).await {
                tracing::debug!("browser message failed: {e}");
            }
        });
    }
}

/// Keep in sync with crates/bridge/src/main.rs.
#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => std::path::PathBuf::from(dir).join("net.spacie.klyxdown.sock"),
        None => std::env::temp_dir().join(format!(
            "net.spacie.klyxdown-{}.sock",
            std::env::var("USER").unwrap_or_else(|_| "user".into())
        )),
    }
}

async fn handle(mut conn: impl AsyncRead + AsyncWrite + Unpin, app: &AppHandle) -> io::Result<()> {
    let mut len = [0u8; 4];
    conn.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::other("message too large"));
    }
    let mut msg = vec![0; len];
    conn.read_exact(&mut msg).await?;

    let reply = match serde_json::from_slice::<Incoming>(&msg) {
        Ok(m) => dispatch(app, m).await,
        Err(e) => Reply::err(format!("KlyxDown did not understand the message: {e}")),
    };
    let bytes = serde_json::to_vec(&reply)?;
    conn.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    conn.write_all(&bytes).await?;
    conn.flush().await
}

async fn dispatch(app: &AppHandle, msg: Incoming) -> Reply {
    if !app
        .state::<AppState>()
        .extension_seen
        .swap(true, Ordering::SeqCst)
    {
        let _ = app.emit("extension-seen", ());
    }
    match msg {
        Incoming::Ping => Reply::ok(),
        Incoming::Show => {
            tray::show_main(app);
            Reply::ok()
        }
        Incoming::Download {
            url,
            filename,
            size,
            referrer,
            cookies,
            user_agent,
            browser,
        } => {
            if !is_web_url(&url) {
                return Reply::err("Only http and https links can be downloaded.");
            }
            let mut captured = Captured {
                id: 0,
                url,
                name: filename.filter(|n| !n.trim().is_empty()),
                size,
                headers: request_headers(referrer, cookies, user_agent),
                browser: browser.unwrap_or_else(|| "Your browser".into()),
            };
            let state = app.state::<AppState>();
            let (ask, dir, connections) = {
                let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
                let name = captured
                    .name
                    .clone()
                    .unwrap_or_else(|| crate::settings::name_from_url(&captured.url));
                (s.ask_on_capture, s.folder_for(&name), s.connections)
            };
            if ask {
                // Hold the reply until the user decides, so Cancel can give the download back.
                let (id, decision) = state.decisions.register();
                captured.id = id;
                deliver_capture(app, captured);
                return match tokio::time::timeout(DECISION_TIMEOUT, decision).await {
                    Ok(Ok(true)) => Reply::ok(),
                    Ok(Ok(false)) => Reply::declined(),
                    _ => {
                        state.decisions.answer(id, false);
                        let _ = app.emit("capture-expired", id);
                        Reply::declined()
                    }
                };
            } else {
                let engine = state.engine.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let req = AddRequest {
                        url: captured.url,
                        dir,
                        file_name: captured.name,
                        connections,
                        start: true,
                        headers: captured.headers,
                    };
                    if let Err(e) = engine.add(req).await {
                        let _ = app.emit("notice", e.to_string());
                    }
                });
            }
            Reply::ok()
        }
        Incoming::Links {
            urls,
            title,
            referrer,
            cookies,
            user_agent,
            browser,
        } => {
            let mut seen = std::collections::HashSet::new();
            let urls: Vec<String> = urls
                .into_iter()
                .filter(|u| is_web_url(u) && seen.insert(u.clone()))
                .take(MAX_LINKS)
                .collect();
            if urls.is_empty() {
                return Reply::err("No downloadable links found.");
            }
            let links = CapturedLinks {
                urls,
                title,
                headers: request_headers(referrer, cookies, user_agent),
                browser: browser.unwrap_or_else(|| "Your browser".into()),
            };
            deliver(app, "capture-links", links, |p, l| p.links = Some(l));
            Reply::ok()
        }
    }
}

/// A link passed on the command line (`klyxdown https://...`) or to a second launch.
pub fn capture_from_args(app: &AppHandle, args: &[String]) -> bool {
    let Some(url) = args.iter().find(|a| is_web_url(a)) else {
        return false;
    };
    let captured = Captured {
        id: 0,
        url: url.clone(),
        name: None,
        size: None,
        headers: Vec::new(),
        browser: "Command line".into(),
    };
    deliver_capture(app, captured);
    true
}

/// A newer parked capture replaces an older one, which goes back to its browser.
pub fn deliver_capture(app: &AppHandle, captured: Captured) {
    let decisions = &app.state::<AppState>().decisions;
    deliver(app, "capture", captured, |p, c| {
        if let Some(old) = p.capture.replace(c) {
            decisions.answer(old.id, false);
        }
    });
}

/// Sends a hand-off to the UI, or parks it until the UI has loaded (cold start by the helper).
fn deliver<T: Serialize + Clone>(
    app: &AppHandle,
    event: &str,
    payload: T,
    park: impl FnOnce(&mut Pending, T),
) {
    let state = app.state::<AppState>();
    if state.ui_ready.load(Ordering::SeqCst) {
        tray::show_main(app);
        let _ = app.emit(event, &payload);
    } else {
        park(
            &mut state.pending.lock().unwrap_or_else(|e| e.into_inner()),
            payload,
        );
        state.show_requested.store(true, Ordering::SeqCst);
    }
}

fn is_web_url(url: &str) -> bool {
    url.len() <= MAX_URL && (url.starts_with("https://") || url.starts_with("http://"))
}

/// Headers that let logged-in downloads work. Values with line breaks are dropped (header injection).
fn request_headers(
    referrer: Option<String>,
    cookies: Option<String>,
    user_agent: Option<String>,
) -> Vec<(String, String)> {
    [
        ("Referer", referrer),
        ("Cookie", cookies),
        ("User-Agent", user_agent),
    ]
    .into_iter()
    .filter_map(|(k, v)| {
        v.filter(|v| !v.is_empty() && v.len() <= MAX_HEADER && !v.contains(['\r', '\n']))
            .map(|v| (k.to_string(), v))
    })
    .collect()
}

/// Writes the host manifests and points every installed browser at them.
/// Idempotent and cheap, so it runs on every start (handles moved installs too).
pub fn register_native_host(app: &AppHandle) {
    if let Err(e) = try_register(app) {
        tracing::warn!("could not register the browser helper: {e}");
    }
}

fn try_register(app: &AppHandle) -> io::Result<()> {
    let helper = std::env::current_exe()?.with_file_name(if cfg!(windows) {
        "klyxdown-bridge.exe"
    } else {
        "klyxdown-bridge"
    });
    if !helper.exists() {
        tracing::info!(path = %helper.display(), "browser helper not built; skipping registration");
        return Ok(());
    }
    let dir = crate::system::data_dir(app)?.join("native-messaging");
    fs::create_dir_all(&dir)?;
    let manifest = |key: &str, value: serde_json::Value| {
        let mut m = json!({
            "name": HOST_NAME,
            "description": "KlyxDown browser helper",
            "path": helper,
            "type": "stdio",
        });
        m[key] = value;
        serde_json::to_vec_pretty(&m)
    };
    let chrome = dir.join("chrome.json");
    let firefox = dir.join("firefox.json");
    fs::write(
        &chrome,
        manifest(
            "allowed_origins",
            json!([format!("chrome-extension://{CHROME_EXTENSION_ID}/")]),
        )?,
    )?;
    fs::write(
        &firefox,
        manifest("allowed_extensions", json!([FIREFOX_EXTENSION_ID]))?,
    )?;
    register_paths(app, &chrome, &firefox)?;
    tracing::info!("browser helper registered");
    Ok(())
}

#[cfg(windows)]
fn register_paths(_app: &AppHandle, chrome: &Path, firefox: &Path) -> io::Result<()> {
    use windows_registry::CURRENT_USER;
    const CHROMIUM: &[&str] = &[
        "Google\\Chrome",
        "Microsoft\\Edge",
        "BraveSoftware\\Brave-Browser",
        "Chromium",
        "Vivaldi",
    ];
    let set = |browser: &str, manifest: &Path| {
        CURRENT_USER
            .create(format!(
                "Software\\{browser}\\NativeMessagingHosts\\{HOST_NAME}"
            ))
            .and_then(|key| key.set_string("", manifest.to_string_lossy()))
            .map_err(io::Error::other)
    };
    for browser in CHROMIUM {
        set(browser, chrome)?;
    }
    set("Mozilla", firefox)
}

#[cfg(unix)]
fn register_paths(app: &AppHandle, chrome: &Path, firefox: &Path) -> io::Result<()> {
    let home = app.path().home_dir().map_err(io::Error::other)?;
    #[cfg(target_os = "macos")]
    let (chromium, mozilla) = (
        [
            "Google/Chrome",
            "Microsoft Edge",
            "BraveSoftware/Brave-Browser",
            "Chromium",
            "Vivaldi",
        ]
        .map(|b| home.join("Library/Application Support").join(b)),
        home.join("Library/Application Support/Mozilla"),
    );
    #[cfg(not(target_os = "macos"))]
    let (chromium, mozilla) = (
        [
            "google-chrome",
            "microsoft-edge",
            "BraveSoftware/Brave-Browser",
            "chromium",
            "vivaldi",
        ]
        .map(|b| home.join(".config").join(b)),
        home.join(".mozilla"),
    );
    let file = format!("{HOST_NAME}.json");
    // Only browsers that are installed (their profile folder exists).
    for root in chromium.iter().filter(|r| r.exists()) {
        let dir = root.join("NativeMessagingHosts");
        fs::create_dir_all(&dir)?;
        fs::copy(chrome, dir.join(&file))?;
    }
    if mozilla.exists() {
        #[cfg(target_os = "macos")]
        let dir = mozilla.join("NativeMessagingHosts");
        #[cfg(not(target_os = "macos"))]
        let dir = mozilla.join("native-messaging-hosts");
        fs::create_dir_all(&dir)?;
        fs::copy(firefox, dir.join(&file))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extension_messages() {
        let m: Incoming = serde_json::from_str(
            r#"{"type":"download","url":"https://x.test/a.zip","userAgent":"UA"}"#,
        )
        .unwrap();
        assert!(matches!(m, Incoming::Download { user_agent: Some(ref ua), .. } if ua == "UA"));
        let m: Incoming =
            serde_json::from_str(r#"{"type":"links","urls":["https://x.test/1"]}"#).unwrap();
        assert!(matches!(m, Incoming::Links { ref urls, .. } if urls.len() == 1));
        assert!(serde_json::from_str::<Incoming>(r#"{"type":"format-disk"}"#).is_err());
    }

    #[test]
    fn only_web_links_pass() {
        assert!(is_web_url("https://example.com/file.zip"));
        assert!(!is_web_url("file:///C:/Windows/system.ini"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url(&format!(
            "https://x.test/{}",
            "a".repeat(MAX_URL)
        )));
    }

    #[test]
    fn decisions_reach_the_waiting_browser_once() {
        let d = Decisions::default();
        let (id, mut rx) = d.register();
        d.answer(id, true);
        d.answer(id, false); // already answered: ignored
        assert_eq!(rx.try_recv(), Ok(true));
        let (id2, mut rx2) = d.register();
        assert_ne!(id, id2);
        drop(d); // closing the app drops the sender: no answer, the browser keeps its copy
        assert!(rx2.try_recv().is_err());
    }

    #[test]
    fn header_values_with_line_breaks_are_dropped() {
        let h = request_headers(
            Some("https://ok.test/".into()),
            Some("a=1\r\nX-Evil: 1".into()),
            None,
        );
        assert_eq!(
            h,
            vec![("Referer".to_string(), "https://ok.test/".to_string())]
        );
    }
}
