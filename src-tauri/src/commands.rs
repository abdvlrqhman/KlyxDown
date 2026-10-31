use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

use klyxdown_engine::{
    AddRequest, DownloadView, Id, Probe, Queue, QueueId, SpeedMode, State as DlState, Status,
    checksum::{self, Algo},
    metalink,
};
use tauri::{AppHandle, Emitter, State, ipc::Channel};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    AppState,
    bridge::{Captured, CapturedLinks, CapturedMedia},
    settings::{self, Settings},
    tray,
};

/// One error type at the IPC edge; the UI receives its message as a string.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Engine(#[from] klyxdown_engine::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Msg(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

type Res<T> = Result<T, AppError>;

#[tauri::command]
/// `headers` carries the browser's cookies and referrer for downloads caught by the extension.
pub async fn probe(
    state: State<'_, AppState>,
    url: String,
    headers: Option<Vec<(String, String)>>,
) -> Res<Probe> {
    Ok(state
        .engine
        .probe_with(&url, &headers.unwrap_or_default())
        .await?)
}

#[tauri::command]
/// An empty `dir` means "the download folder, sorted by type" (used for batches from the browser).
pub async fn add(state: State<'_, AppState>, mut req: AddRequest) -> Res<Id> {
    tracing::info!(url = %req.url, "add");
    let s = lock(&state).clone();
    req.seed = s.seed;
    if req.dir.as_os_str().is_empty() {
        req.dir = match &req.media {
            // Videos and songs sort by what they are; their names come later.
            Some(m) if s.sort_by_type => {
                s.download_dir
                    .join(if m.audio.is_some() { "Music" } else { "Video" })
            }
            Some(_) => s.download_dir.clone(),
            None => {
                let name = req
                    .file_name
                    .clone()
                    .unwrap_or_else(|| settings::name_from_url(&req.url));
                s.folder_for(&name)
            }
        };
    }
    Ok(state.engine.add(req).await?)
}

/// Where a file with this name would be saved, so the add panel can show it.
#[tauri::command]
pub fn suggest_dir(state: State<'_, AppState>, name: String) -> PathBuf {
    lock(&state).folder_for(&name)
}

/// Checksum of a finished file, computed off the UI thread.
#[tauri::command]
pub async fn file_hash(state: State<'_, AppState>, id: Id, algo: Algo) -> Res<String> {
    let path = file_of(&state, id)?;
    tauri::async_runtime::spawn_blocking(move || Ok(checksum::file_hash(&path, algo)?))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))?
}

/// `https://x.test/photo[001-120].jpg` as its links; empty when it is a plain link.
#[tauri::command]
pub fn expand_batch(pattern: String) -> Res<Vec<String>> {
    match klyxdown_engine::batch::expand(pattern.trim()) {
        None => Ok(Vec::new()),
        Some(result) => result.map_err(AppError::Msg),
    }
}

/// The files a Metalink describes, from a file on disk or a link.
#[tauri::command]
pub async fn read_metalink(
    state: State<'_, AppState>,
    source: String,
) -> Res<Vec<metalink::MetalinkFile>> {
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let text = if source.starts_with("http://") || source.starts_with("https://") {
        String::from_utf8_lossy(&state.engine.fetch_small(&source, MAX_BYTES).await?).into_owned()
    } else {
        let path = PathBuf::from(&source);
        if std::fs::metadata(&path)?.len() > MAX_BYTES {
            return Err(AppError::Msg(
                "That file is too large to be a Metalink.".into(),
            ));
        }
        String::from_utf8_lossy(&std::fs::read(path)?).into_owned()
    };
    metalink::parse(&text).map_err(AppError::Msg)
}

/// What is inside a remote ZIP.
#[tauri::command]
pub async fn list_zip(
    state: State<'_, AppState>,
    url: String,
    headers: Option<Vec<(String, String)>>,
) -> Res<Vec<klyxdown_engine::zip::ZipEntry>> {
    Ok(state
        .engine
        .list_zip(&url, &headers.unwrap_or_default())
        .await?)
}

/// A fresh link pasted for a download whose link expired.
#[tauri::command]
pub async fn refresh_link(state: State<'_, AppState>, id: Id, url: String) -> Res<()> {
    Ok(state
        .engine
        .refresh_link(id, url.trim(), Vec::new())
        .await?)
}

/// Opens the page a download came from, and takes the next matching download the browser
/// hands over (same size or name) as that download's fresh link.
#[tauri::command]
pub fn wait_for_link(app: AppHandle, state: State<'_, AppState>, id: Id) -> Res<()> {
    let d = state
        .engine
        .list()
        .into_iter()
        .find(|d| d.id == id)
        .ok_or(klyxdown_engine::Error::NotFound(id))?;
    let page = d
        .page
        .clone()
        .filter(|p| p.starts_with("https://") || p.starts_with("http://"))
        .ok_or_else(|| {
            AppError::Msg(
                "This download has no page to go back to. Paste a fresh link instead.".into(),
            )
        })?;
    *state.refresh_wait.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(crate::bridge::RefreshWait::new(&d));
    app.opener()
        .open_url(page, None::<&str>)
        .map_err(|e| AppError::Msg(format!("Could not open the browser: {e}")))
}

/// The user decided about a download caught from the browser: take it, or give it back.
#[tauri::command]
pub fn capture_done(state: State<'_, AppState>, id: u64, taken: bool) {
    state.decisions.answer(id, taken);
}

/// Whether the browser extension has talked to the app since it started.
#[tauri::command]
pub fn extension_seen(state: State<'_, AppState>) -> bool {
    state.extension_seen.load(Ordering::SeqCst)
}

/// "nothing", "quit", "sleep" or "shutdown" once every download has finished.
#[tauri::command]
pub fn set_finish_action(state: State<'_, AppState>, action: String) -> &'static str {
    state.finish_action.set(&action);
    state.finish_action.get()
}

/// Called by the UI when the countdown reaches zero (the user did not cancel).
#[tauri::command]
pub fn run_finish_action(app: AppHandle, action: String) -> Res<()> {
    Ok(crate::system::run_finish_action(&app, &action)?)
}

#[tauri::command]
pub fn is_portable() -> bool {
    crate::system::portable_dir().is_some()
}

/// A newer version published on GitHub Releases.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    current: String,
    notes: Option<String>,
}

/// Asks GitHub Releases (the `latest.json` the release workflow publishes) for a newer version.
#[tauri::command]
pub async fn check_update(app: AppHandle, state: State<'_, AppState>) -> Res<Option<UpdateInfo>> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| AppError::Msg(e.to_string()))?
        .check()
        .await
        .map_err(|e| AppError::Msg(format!("Could not check for updates: {e}")))?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone(),
    });
    *state.update.lock().unwrap_or_else(|e| e.into_inner()) = update;
    Ok(info)
}

/// Downloads the update (signature checked by the plugin), saves every download's
/// progress, installs, and restarts into the new version.
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, AppState>) -> Res<()> {
    let update = state
        .update
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or_else(|| AppError::Msg("Check for updates first.".into()))?;
    let progress = app.clone();
    let engine = state.engine.clone();
    let mut received = 0u64;
    update
        .download_and_install(
            |chunk, total| {
                received += chunk as u64;
                let _ = progress.emit("update-progress", (received, total));
            },
            // On Windows the installer closes the app right away, so save first.
            move || engine.shutdown(),
        )
        .await
        .map_err(|e| AppError::Msg(format!("The update could not be installed: {e}")))?;
    app.restart()
}

pub const PROJECT_PAGE: &str = "https://spacie.net/klyxdown";

/// Opens spacie.net/klyxdown in the default browser (fixed URL, nothing from the UI).
#[tauri::command]
pub fn open_project_page(app: AppHandle) -> Res<()> {
    app.opener()
        .open_url(PROJECT_PAGE, None::<&str>)
        .map_err(|e| AppError::Msg(format!("Could not open the browser: {e}")))
}

/// Reads a link list: IDM's `.ef2` export or any text file with one link per line.
#[tauri::command]
pub fn import_links(path: PathBuf) -> Res<Vec<String>> {
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    if std::fs::metadata(&path)?.len() > MAX_BYTES {
        return Err(AppError::Msg(
            "That file is too large to be a link list.".into(),
        ));
    }
    let text = String::from_utf8_lossy(&std::fs::read(&path)?).into_owned();
    Ok(parse_links(&text))
}

/// `.ef2` wraps each link in `<` and `>` lines with headers after it; plain lists are just links.
fn parse_links(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
        .filter(|l| seen.insert(l.to_string()))
        .map(String::from)
        .collect()
}

#[tauri::command]
pub fn pause(state: State<'_, AppState>, id: Id) -> Res<()> {
    Ok(state.engine.pause(id)?)
}

#[tauri::command]
pub fn resume(state: State<'_, AppState>, id: Id) -> Res<()> {
    Ok(state.engine.resume(id)?)
}

/// Throws away the progress and downloads the file again (for example after it changed on the server).
#[tauri::command]
pub fn restart(state: State<'_, AppState>, id: Id) -> Res<()> {
    Ok(state.engine.restart(id)?)
}

#[tauri::command]
pub fn remove(state: State<'_, AppState>, id: Id, delete_file: bool) -> Res<()> {
    Ok(state.engine.remove(id, delete_file)?)
}

#[tauri::command]
pub fn pause_all(state: State<'_, AppState>) {
    state.engine.pause_all();
}

#[tauri::command]
pub fn resume_all(state: State<'_, AppState>) {
    state.engine.resume_all();
}

#[tauri::command]
pub fn list(state: State<'_, AppState>) -> Vec<DownloadView> {
    state.engine.list()
}

/// What the UI redraws 4 times a second.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    downloads: Vec<DownloadView>,
    status: Status,
    /// The connection is metered (whether or not downloads are held for it).
    metered: bool,
}

/// Streams the whole list to the UI 4 times a second. A newer call (for example
/// after a webview reload) stops the older stream.
#[tauri::command]
pub fn watch(app: AppHandle, state: State<'_, AppState>, on_progress: Channel<Snapshot>) {
    let generation = state.watch_gen.fetch_add(1, Ordering::SeqCst) + 1;
    tracing::info!(generation, "UI connected, streaming progress");
    let engine = state.engine.clone();
    let current = state.watch_gen.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        while current.load(Ordering::SeqCst) == generation {
            tick.tick().await;
            let snapshot = Snapshot {
                downloads: engine.list(),
                status: engine.status(),
                metered: crate::system::metered(&app),
            };
            if on_progress.send(snapshot).is_err() {
                break;
            }
        }
    });
}

#[tauri::command]
pub fn queues(state: State<'_, AppState>) -> Vec<Queue> {
    state.engine.queues()
}

#[tauri::command]
pub fn add_queue(state: State<'_, AppState>, queue: Queue) -> QueueId {
    state.engine.add_queue(queue)
}

/// yt-dlp and ffmpeg: installed or not, and versions.
#[tauri::command]
pub async fn tools_status(state: State<'_, AppState>) -> Res<klyxdown_engine::tools::ToolStatus> {
    Ok(state.engine.tools().await)
}

/// Downloads yt-dlp (and ffmpeg on Windows and macOS). The UI asks first.
#[tauri::command]
pub async fn install_tools(state: State<'_, AppState>) -> Res<klyxdown_engine::tools::ToolStatus> {
    state.engine.install_tools().await?;
    Ok(state.engine.tools().await)
}

/// The qualities a page's video comes in.
#[tauri::command]
pub async fn media_info(
    state: State<'_, AppState>,
    url: String,
    headers: Option<Vec<(String, String)>>,
) -> Res<klyxdown_engine::media::MediaInfo> {
    Ok(state
        .engine
        .media_info(url.trim(), &headers.unwrap_or_default())
        .await?)
}

/// The files in a magnet link, `.torrent` link or `.torrent` file.
#[tauri::command]
pub async fn inspect_torrent(
    state: State<'_, AppState>,
    source: String,
) -> Res<klyxdown_engine::torrent::TorrentInfo> {
    Ok(state.engine.inspect_torrent(source.trim()).await?)
}

#[tauri::command]
pub fn convert_presets() -> Vec<&'static str> {
    klyxdown_engine::convert::presets()
}

/// Converts a finished download; returns the new job's id.
#[tauri::command]
pub fn convert(state: State<'_, AppState>, id: Id, preset: String) -> Res<Id> {
    Ok(state.engine.convert(id, &preset)?)
}

#[tauri::command]
pub fn set_seeding(state: State<'_, AppState>, id: Id, seed: bool) -> Res<()> {
    Ok(state.engine.set_seeding(id, seed)?)
}

/// Where the in-app player reads a download from while it downloads.
#[tauri::command]
pub fn preview_url(state: State<'_, AppState>, id: Id) -> Res<String> {
    state
        .preview
        .as_ref()
        .map(|p| p.url(id))
        .ok_or_else(|| AppError::Msg("The preview player could not start.".into()))
}

/// The main queue's concurrency is the "downloads at the same time" setting, so it is saved there.
#[tauri::command]
pub fn update_queue(app: AppHandle, state: State<'_, AppState>, queue: Queue) -> Res<()> {
    if queue.id == klyxdown_engine::MAIN_QUEUE {
        let max = queue.max_active.clamp(1, 16);
        update_settings(&app, &state, |s| s.max_active = max)?;
    }
    Ok(state.engine.update_queue(queue)?)
}

#[tauri::command]
pub fn remove_queue(state: State<'_, AppState>, id: QueueId) -> Res<()> {
    Ok(state.engine.remove_queue(id)?)
}

#[tauri::command]
pub fn move_to_queue(state: State<'_, AppState>, id: Id, queue: QueueId) -> Res<()> {
    Ok(state.engine.move_to_queue(id, queue)?)
}

/// Download on the current metered connection anyway, until the connection changes.
#[tauri::command]
pub fn allow_metered(state: State<'_, AppState>) {
    state.metered_allowed.store(true, Ordering::SeqCst);
    state.engine.set_metered_hold(false);
}

/// Tray menu and tooltip in the interface language.
#[tauri::command]
pub fn set_tray_text(app: AppHandle, text: tray::TrayText) -> Res<()> {
    tray::set_text(&app, text).map_err(|e| AppError::Msg(e.to_string()))
}

#[tauri::command]
pub fn set_mode(app: AppHandle, state: State<'_, AppState>, mode: SpeedMode) -> Res<()> {
    state.engine.set_mode(mode);
    update_settings(&app, &state, |s| s.mode = mode)
}

#[tauri::command]
pub fn set_limit(app: AppHandle, state: State<'_, AppState>, limit: Option<u64>) -> Res<()> {
    let limit = limit.filter(|l| *l >= 16 * 1024);
    state.engine.set_limit(limit);
    update_settings(&app, &state, |s| s.limit = limit)
}

/// The webview knows about OS network changes first; it reports them here.
#[tauri::command]
pub fn network_changed(app: AppHandle, state: State<'_, AppState>, online: bool) {
    state.online.store(online, Ordering::Relaxed);
    state.engine.set_online(online);
    tray::refresh_status(&app);
}

#[tauri::command]
pub fn get_settings(app: AppHandle, state: State<'_, AppState>) -> Settings {
    let mut s = lock(&state).clone();
    // The OS is the source of truth for autostart; the user may have changed it there.
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        s.run_on_startup = enabled;
    }
    s
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Res<Settings> {
    let next = settings.sanitized(&app);
    std::fs::create_dir_all(&next.download_dir)?;

    // Proxy first: a rejected address must not leave other changes half-applied.
    // Only rebuild the HTTP client when the proxy really changed (it drops the connection pool).
    if next.proxy() != lock(&state).proxy() {
        state.engine.set_proxy(next.proxy())?;
    }

    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().unwrap_or(false) != next.run_on_startup {
        let result = if next.run_on_startup {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        result.map_err(|e| AppError::Msg(format!("Could not change run on startup: {e}")))?;
    }

    state.engine.set_max_active(next.max_active);
    state.engine.set_mode(next.mode);
    state.engine.set_limit(next.limit);
    state.engine.set_quota(next.quota());
    state.engine.set_seed_ratio(next.seed_ratio);
    if next.handle_magnets != lock(&state).handle_magnets {
        crate::system::set_magnet_handler(&app, next.handle_magnets);
    }
    settings::save(&app, &next)?;
    *lock(&state) = next.clone();
    Ok(next)
}

/// What the UI needs when it has loaded: whether to show the window, and any browser
/// hand-off that arrived while it was loading.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiStart {
    show: bool,
    capture: Option<Captured>,
    links: Option<CapturedLinks>,
    media: Option<CapturedMedia>,
}

#[tauri::command]
pub fn ui_ready(state: State<'_, AppState>) -> UiStart {
    state.ui_ready.store(true, Ordering::SeqCst);
    let mut pending = state.pending.lock().unwrap_or_else(|e| e.into_inner());
    let (capture, links, media) = (
        pending.capture.take(),
        pending.links.take(),
        pending.media.take(),
    );
    let requested = state.show_requested.swap(false, Ordering::SeqCst);
    UiStart {
        show: !state.launched_hidden
            || requested
            || capture.is_some()
            || links.is_some()
            || media.is_some(),
        capture,
        links,
        media,
    }
}

#[tauri::command]
pub fn open_file(app: AppHandle, state: State<'_, AppState>, id: Id) -> Res<()> {
    let path = file_of(&state, id)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::Msg(format!("Could not open the file: {e}")))
}

#[tauri::command]
pub fn reveal_file(app: AppHandle, state: State<'_, AppState>, id: Id) -> Res<()> {
    let path = file_of(&state, id)?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| AppError::Msg(format!("Could not show the file: {e}")))
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}

/// Paths come from the engine, never from the webview, so the UI cannot open arbitrary files.
fn file_of(state: &AppState, id: Id) -> Res<PathBuf> {
    let d = state
        .engine
        .list()
        .into_iter()
        .find(|d| d.id == id)
        .ok_or(klyxdown_engine::Error::NotFound(id))?;
    if d.state == DlState::Done {
        return Ok(d.path);
    }
    let mut partial = d.path.into_os_string();
    partial.push(".kdpart");
    Ok(partial.into())
}

fn lock(state: &AppState) -> std::sync::MutexGuard<'_, Settings> {
    state.settings.lock().unwrap_or_else(|e| e.into_inner())
}

fn update_settings(
    app: &AppHandle,
    state: &AppState,
    change: impl FnOnce(&mut Settings),
) -> Res<()> {
    let mut s = lock(state);
    change(&mut s);
    settings::save(app, &s)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_links;

    #[test]
    fn reads_idm_ef2_exports_and_plain_lists() {
        let ef2 = "<
https://a.test/file.zip
referer: https://a.test/
User-Agent: X
>
<
https://b.test/x.iso
>
";
        assert_eq!(
            parse_links(ef2),
            ["https://a.test/file.zip", "https://b.test/x.iso"]
        );
        let plain = "https://a.test/1

not a link
ftp://old.test/f
https://a.test/1
";
        assert_eq!(parse_links(plain), ["https://a.test/1"]);
    }
}
