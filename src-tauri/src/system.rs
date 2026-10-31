//! Background watcher for things outside the app: keeps the computer awake while
//! downloading, offers links copied to the clipboard, and announces the
//! "when everything finishes" action. Runs on one dedicated thread, which the
//! Windows keep-awake call requires (its state belongs to the calling thread).

use std::{
    io,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
    thread,
    time::Duration,
};

use klyxdown_engine::{DownloadView, State};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::{AppState, bridge, settings, tray};

/// What to do once every download has finished. Not saved: it resets to nothing
/// on each launch, so a forgotten "shut down" can never surprise anyone.
#[derive(Default)]
pub struct FinishAction(AtomicU8);

impl FinishAction {
    pub const ACTIONS: [&str; 4] = ["nothing", "quit", "sleep", "shutdown"];

    pub fn set(&self, action: &str) {
        let i = Self::ACTIONS.iter().position(|a| *a == action).unwrap_or(0);
        self.0.store(i as u8, Ordering::SeqCst);
    }

    pub fn get(&self) -> &'static str {
        Self::ACTIONS[self.0.load(Ordering::SeqCst) as usize]
    }

    fn take(&self) -> &'static str {
        Self::ACTIONS[self.0.swap(0, Ordering::SeqCst) as usize]
    }
}

pub fn start(app: AppHandle) {
    thread::spawn(move || watch(app));
}

/// Last metered check (refreshed every few seconds by the watcher thread).
static METERED: AtomicBool = AtomicBool::new(false);

pub fn metered(_app: &AppHandle) -> bool {
    METERED.load(Ordering::Relaxed)
}

fn watch(app: AppHandle) {
    let mut awake = false;
    let mut had_work = false;
    let mut clipboard_on = false;
    let mut last_clip = String::new();
    let mut held = false;
    let mut second = 0u64;
    loop {
        thread::sleep(Duration::from_secs(1));
        second += 1;
        let state = app.state::<AppState>();
        let (keep_awake, watch_clipboard, pause_on_metered) = {
            let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
            (s.keep_awake, s.watch_clipboard, s.pause_on_metered)
        };

        if second % 10 == 1 {
            let now = is_metered();
            // A different connection: an earlier "download anyway" no longer applies.
            if METERED.swap(now, Ordering::Relaxed) != now {
                state.metered_allowed.store(false, Ordering::SeqCst);
            }
        }
        let hold = pause_on_metered
            && METERED.load(Ordering::Relaxed)
            && !state.metered_allowed.load(Ordering::SeqCst);
        if hold != held {
            state.engine.set_metered_hold(hold);
            held = hold;
        }

        let list = state.engine.list();
        let busy = list
            .iter()
            .any(|d| matches!(d.state, State::Downloading | State::WaitingForNetwork));
        let pending = has_work(&list);

        if keep_awake && busy != awake {
            set_awake(busy);
            awake = busy;
        } else if !keep_awake && awake {
            set_awake(false);
            awake = false;
        }

        // Fires only right after something completed: pausing or removing the last running
        // download, or a failure, must never start a shutdown.
        if had_work && !pending && just_finished(&list, now_ms()) {
            let action = state.finish_action.take();
            if action != "nothing" {
                tray::show_main(&app);
                let _ = app.emit("finish-countdown", action);
            }
        }
        had_work = pending;

        if watch_clipboard {
            // A failed read (another app holds the clipboard) is not a change.
            let Ok(text) = app.clipboard().read_text() else {
                continue;
            };
            // Whatever was on the clipboard before watching started is not offered.
            if clipboard_on
                && text != last_clip
                && let Some(url) = downloadable_link(&text, &list)
            {
                let captured = bridge::Captured {
                    id: 0,
                    url,
                    name: None,
                    size: None,
                    headers: Vec::new(),
                    browser: "Clipboard".into(),
                };
                bridge::deliver_capture(&app, captured);
            }
            last_clip = text;
        }
        clipboard_on = watch_clipboard;
    }
}

/// Downloads that still need attention before "everything finished" applies.
fn has_work(list: &[DownloadView]) -> bool {
    list.iter().any(|d| {
        matches!(
            d.state,
            State::Downloading | State::WaitingForNetwork | State::Queued | State::Paused
        )
    })
}

/// A download completed in the last 15 seconds.
fn just_finished(list: &[DownloadView], now: u64) -> bool {
    list.iter().any(|d| {
        d.state == State::Done
            && d.finished_at
                .is_some_and(|t| now.saturating_sub(t) < 15_000)
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A single http(s) link to a recognizable file that is not already in the list.
fn downloadable_link(text: &str, list: &[DownloadView]) -> Option<String> {
    let url = text.trim();
    let single = !url.contains(char::is_whitespace) && url.len() <= 8 * 1024;
    let web = url.starts_with("https://") || url.starts_with("http://");
    let file = settings::category_folder(&settings::name_from_url(url)).is_some();
    let known = list.iter().any(|d| d.url == url);
    (single && web && file && !known).then(|| url.to_string())
}

#[cfg(windows)]
fn set_awake(on: bool) {
    unsafe extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    let flags = if on {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    // SAFETY: plain flag call; it only affects this thread's power request.
    unsafe { SetThreadExecutionState(flags) };
}

/// macOS and Linux: a helper process holds the sleep block (`caffeinate`, `systemd-inhibit`)
/// for as long as it runs. It also ends by itself if KlyxDown exits without letting go.
#[cfg(not(windows))]
fn set_awake(on: bool) {
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::Mutex;
    static HOLD: Mutex<Option<Child>> = Mutex::new(None);
    let mut hold = HOLD.lock().unwrap_or_else(|e| e.into_inner());
    if !on {
        if let Some(mut child) = hold.take() {
            // The whole group: systemd-inhibit and the command it runs.
            // SAFETY: plain signal to a process group we created.
            unsafe { libc::kill(-(child.id() as i32), libc::SIGTERM) };
            let _ = child.wait();
        }
        return;
    }
    if hold.is_some() {
        return;
    }
    let pid = std::process::id().to_string();
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("caffeinate");
        c.args(["-i", "-w", &pid]);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = {
        let mut c = Command::new("systemd-inhibit");
        c.args([
            "--what=sleep:idle",
            "--who=KlyxDown",
            "--why=Downloading",
            "--mode=block",
            "tail",
            &format!("--pid={pid}"),
            "-f",
            "/dev/null",
        ]);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    match cmd.spawn() {
        Ok(child) => *hold = Some(child),
        Err(e) => tracing::info!("can't keep the computer awake here: {e}"),
    }
}

/// Windows reports a cost for the internet connection: fixed or variable plans, roaming,
/// or over the data limit count as metered.
#[cfg(windows)]
fn is_metered() -> bool {
    use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
    let cost = || -> windows::core::Result<bool> {
        let cost = NetworkInformation::GetInternetConnectionProfile()?.GetConnectionCost()?;
        Ok(cost.Roaming()?
            || cost.OverDataLimit()?
            || matches!(
                cost.NetworkCostType()?,
                NetworkCostType::Fixed | NetworkCostType::Variable
            ))
    };
    // No internet profile (offline) is not metered.
    cost().unwrap_or(false)
}

/// NetworkManager's `Metered`: 1 is yes, 3 is "guessed yes" (a phone hotspot), the rest no.
#[cfg(all(unix, not(target_os = "macos")))]
fn is_metered() -> bool {
    std::process::Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "Metered",
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| {
            o.status.success() && matches!(String::from_utf8_lossy(&o.stdout).trim(), "u 1" | "u 3")
        })
}

/// Network.framework marks a path "expensive" (cellular, hotspot) or "constrained" (Low Data Mode).
#[cfg(target_os = "macos")]
fn is_metered() -> bool {
    use std::ffi::{c_char, c_void};
    use std::sync::Once;
    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        fn nw_path_monitor_create() -> *mut c_void;
        fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        fn nw_path_monitor_set_update_handler(
            monitor: *mut c_void,
            handler: &block2::Block<dyn Fn(*mut c_void)>,
        );
        fn nw_path_monitor_start(monitor: *mut c_void);
        fn nw_path_is_expensive(path: *mut c_void) -> bool;
        fn nw_path_is_constrained(path: *mut c_void) -> bool;
    }
    unsafe extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
    }
    static EXPENSIVE: AtomicBool = AtomicBool::new(false);
    static START: Once = Once::new();
    START.call_once(|| {
        let handler = block2::RcBlock::new(|path: *mut c_void| {
            // SAFETY: Network.framework passes a valid path object for the handler's duration.
            let costly = unsafe { nw_path_is_expensive(path) || nw_path_is_constrained(path) };
            EXPENSIVE.store(costly, Ordering::Relaxed);
        });
        // SAFETY: plain C calls; the monitor, its queue and the handler live for the whole
        // process (the monitor copies the block, and none of them is ever released).
        unsafe {
            let monitor = nw_path_monitor_create();
            let queue = dispatch_queue_create(
                c"net.spacie.klyxdown.network".as_ptr(),
                std::ptr::null_mut(),
            );
            nw_path_monitor_set_queue(monitor, queue);
            nw_path_monitor_set_update_handler(monitor, &handler);
            nw_path_monitor_start(monitor);
        }
    });
    EXPENSIVE.load(Ordering::Relaxed)
}

/// Runs the chosen action once the countdown in the UI reaches zero.
pub fn run_finish_action(app: &AppHandle, action: &str) -> io::Result<()> {
    if action == "nothing" {
        return Ok(());
    }
    // Save every download's progress before the computer or the app goes away.
    app.state::<AppState>().engine.shutdown();
    match action {
        "quit" => app.exit(0),
        "sleep" => sleep_now()?,
        "shutdown" => shut_down()?,
        _ => {}
    }
    Ok(())
}

#[cfg(windows)]
fn sleep_now() -> io::Result<()> {
    #[link(name = "powrprof")]
    unsafe extern "system" {
        fn SetSuspendState(hibernate: u8, force: u8, wake_events_disabled: u8) -> u8;
    }
    // SAFETY: documented power API; FALSE, FALSE, FALSE asks for normal sleep.
    if unsafe { SetSuspendState(0, 0, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn sleep_now() -> io::Result<()> {
    std::process::Command::new("pmset")
        .arg("sleepnow")
        .spawn()
        .map(drop)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn sleep_now() -> io::Result<()> {
    std::process::Command::new("systemctl")
        .arg("suspend")
        .spawn()
        .map(drop)
}

fn shut_down() -> io::Result<()> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("shutdown");
        c.args(["/s", "/t", "0"]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("osascript");
        c.args(["-e", "tell application \"System Events\" to shut down"]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("systemctl");
        c.arg("poweroff");
        c
    };
    cmd.spawn().map(drop)
}

/// Applies the "open magnet links" setting at startup, and listens for links macOS hands over.
pub fn register_magnets(app: &AppHandle) {
    use tauri_plugin_deep_link::DeepLinkExt;
    let on = app
        .state::<AppState>()
        .settings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .handle_magnets;
    set_magnet_handler(app, on);
    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
        bridge::capture_from_args(&handle, &urls);
    });
}

/// Windows and Linux register the `magnet:` scheme at runtime; macOS does it through the
/// app bundle (the user picks the default app in their browser or with Finder).
pub fn set_magnet_handler(app: &AppHandle, on: bool) {
    #[cfg(any(windows, target_os = "linux"))]
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        let links = app.deep_link();
        let result = if on {
            links.register("magnet")
        } else {
            links.unregister("magnet")
        };
        if let Err(e) = result {
            tracing::warn!("magnet link registration: {e}");
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = (app, on);
}

/// Portable mode: a file named `portable` next to the program keeps all data
/// (settings, download list, browser helper manifests) in a `data` folder beside it.
pub fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let base = exe.parent()?;
    base.join("portable").exists().then(|| base.join("data"))
}

pub fn data_dir(app: &AppHandle) -> io::Result<PathBuf> {
    match portable_dir() {
        Some(dir) => Ok(dir),
        None => app.path().app_data_dir().map_err(io::Error::other),
    }
}

pub fn config_dir(app: &AppHandle) -> io::Result<PathBuf> {
    match portable_dir() {
        Some(dir) => Ok(dir),
        None => app.path().app_config_dir().map_err(io::Error::other),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn only_new_file_links_are_offered_from_the_clipboard() {
        assert_eq!(
            downloadable_link("  https://x.test/setup.exe \n", &[]).as_deref(),
            Some("https://x.test/setup.exe")
        );
        assert_eq!(downloadable_link("https://x.test/blog/post", &[]), None);
        assert_eq!(downloadable_link("see https://x.test/a.zip", &[]), None);
        assert_eq!(downloadable_link("file:///C:/a.zip", &[]), None);
    }

    pub(crate) fn view(state: State, finished_at: Option<u64>) -> DownloadView {
        DownloadView {
            id: 1,
            url: "https://x.test/a.zip".into(),
            name: "a.zip".into(),
            host: "x.test".into(),
            path: PathBuf::from("a.zip"),
            size: Some(1),
            bytes: 1,
            state,
            resumable: true,
            connections: 1,
            finished_at,
            ..DownloadView::default()
        }
    }

    #[test]
    fn pausing_or_failing_never_counts_as_everything_finished() {
        let now = 100_000;
        assert!(has_work(&[view(State::Paused, None)]));
        assert!(!just_finished(&[view(State::Failed, None)], now));
        assert!(!just_finished(
            &[view(State::Done, Some(now - 60_000))],
            now
        ));
        assert!(just_finished(&[view(State::Done, Some(now - 2_000))], now));
    }

    #[test]
    fn finish_action_is_used_once() {
        let a = FinishAction::default();
        a.set("shutdown");
        assert_eq!(a.get(), "shutdown");
        assert_eq!(a.take(), "shutdown");
        assert_eq!(a.get(), "nothing");
        a.set("format-disk");
        assert_eq!(a.get(), "nothing");
    }
}
