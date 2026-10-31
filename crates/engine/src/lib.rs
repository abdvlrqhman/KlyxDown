//! KlyxDown download engine.
//!
//! Pure Rust, no Tauri. The app talks to it through [`Engine`]: commands in,
//! [`DownloadView`] snapshots and [`Event`]s out.
//!
//! Must be created inside a Tokio runtime ([`Engine::new`] spawns its scheduler).

pub mod batch;
pub mod checksum;
pub mod convert;
mod db;
mod download;
pub mod ftp;
pub mod grab;
mod limiter;
pub mod media;
pub mod metalink;
mod names;
mod probe;
mod queue;
mod runner;
pub mod tools;
pub mod torrent;
mod worker;
pub mod zip;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use download::{Download, Shared, SlowRule, effective_rate, part_path};

pub use checksum::Checksum;
pub use queue::{MAIN_QUEUE, Queue, QueueId, Schedule};

pub type Id = u64;

/// How a download gets its bytes.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A link to a file (HTTP, HTTPS, FTP), fetched in segments.
    #[default]
    Direct,
    /// Video or audio from a web page, through yt-dlp.
    Media,
    /// A magnet link or `.torrent`.
    Torrent,
    /// A finished file being converted by ffmpeg.
    Convert,
}

/// A readable, seekable stream (a torrent file for the preview player).
pub trait ReadSeek: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin + Send> ReadSeek for T {}

/// Why nothing is downloading right now even though downloads are waiting.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Hold {
    /// The connection is metered and the user asked not to download on it.
    Metered,
    /// The download quota for this period is used up.
    Quota {
        /// Unix milliseconds when the next period starts.
        resumes_at: u64,
    },
}

/// "Download no more than `bytes` every `hours`."
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    pub bytes: u64,
    pub hours: u32,
}

/// Engine-wide state the UI shows next to the list.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub hold: Option<Hold>,
    /// Bytes downloaded in the current quota period (0 without a quota).
    pub quota_used: u64,
}

/// Traffic presets the user picks with the speed knob.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpeedMode {
    /// Global cap of 512 KiB/s, at most 2 connections per download.
    Snail,
    /// No mode cap (the user limit still applies), at most 4 connections per download.
    #[default]
    Balanced,
    /// No mode cap, the download's own connection count.
    Full,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum State {
    #[default]
    Queued,
    Downloading,
    Paused,
    /// A connection error happened; retrying with backoff, and immediately on `set_online(true)`.
    WaitingForNetwork,
    Failed,
    Done,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentView {
    pub start: u64,
    pub end: u64,
    pub pos: u64,
    /// Bytes per second over the last couple of seconds.
    pub speed: u64,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadView {
    pub id: Id,
    pub url: String,
    pub name: String,
    pub host: String,
    /// Final file path (the partial file is this path plus `.kdpart`).
    pub path: PathBuf,
    /// `None` when the server does not send a size.
    pub size: Option<u64>,
    pub bytes: u64,
    /// Bytes per second, smoothed.
    pub speed: u64,
    pub state: State,
    /// Human-readable reason when `state` is `Failed` or `WaitingForNetwork`.
    pub error: Option<String>,
    /// Server accepts byte ranges, so pausing and multiple connections are safe.
    pub resumable: bool,
    pub connections: u8,
    /// Sorted by `start`. Finished segments stay in the list so the UI can draw the file map.
    pub segments: Vec<SegmentView>,
    /// Unix milliseconds.
    pub added_at: u64,
    pub finished_at: Option<u64>,
    pub kind: Kind,
    pub queue: QueueId,
    /// The server refused the link (expired or one-time): a fresh link from the page fixes it.
    pub needs_link: bool,
    /// Other servers with the same file, used alongside `url`.
    pub mirrors: usize,
    /// The web page the download came from (its referrer), where a fresh link can be found.
    pub page: Option<String>,
    /// What it is doing besides downloading ("Merging audio and video", "Converting").
    pub detail: Option<String>,
    /// Torrents: sharing speed, total shared, connected people, and still sharing when done.
    pub upload_speed: u64,
    pub uploaded: u64,
    pub peers: u32,
    pub seeding: bool,
}

/// What the server told us about a link before downloading.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    /// URL after redirects.
    pub url: String,
    pub name: String,
    pub host: String,
    pub size: Option<u64>,
    pub resumable: bool,
    pub mime: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddRequest {
    pub url: String,
    pub dir: PathBuf,
    /// Overrides the probed name.
    pub file_name: Option<String>,
    /// 1 to 32. Forced to 1 when the server is not resumable.
    pub connections: u8,
    /// `false` puts it in the queue as `Paused`.
    pub start: bool,
    /// Extra request headers (cookies, referrer) handed over by the browser extension.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Missing or unknown queues mean the main queue.
    #[serde(default)]
    pub queue: QueueId,
    /// Other links to the same file. Those that answer with the same size share the work.
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// Checked once the file is complete; a mismatch fails the download.
    #[serde(default)]
    pub checksum: Option<Checksum>,
    /// Download only this file (its path inside the archive) from a remote ZIP.
    #[serde(default)]
    pub zip_entry: Option<String>,
    /// Video or audio from the page at `url`, in this quality (see [`Engine::media_info`]).
    #[serde(default)]
    pub media: Option<media::MediaChoice>,
    /// For a magnet or `.torrent` in `url`: which files (see [`Engine::inspect_torrent`]).
    #[serde(default)]
    pub torrent_files: Option<Vec<usize>>,
    /// Keep sharing the torrent once it is complete.
    #[serde(default)]
    pub seed: bool,
    /// Convert the file with this preset once it is complete (see [`convert::presets`]).
    #[serde(default)]
    pub convert: Option<String>,
}

/// Which proxy downloads go through.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind", content = "url")]
pub enum Proxy {
    /// The operating system's proxy settings (and `HTTPS_PROXY` style variables).
    #[default]
    System,
    /// Connect directly.
    None,
    /// `http://`, `https://`, `socks5://` or `socks5h://`, optionally with `user:pass@`.
    Custom(String),
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Downloads from the main queue running at the same time; the rest wait as `Queued`.
    pub max_active: usize,
    pub mode: SpeedMode,
    /// User speed limit in bytes per second, applied on top of the mode.
    pub limit: Option<u64>,
    pub user_agent: String,
    /// SQLite file the downloads are kept in across restarts. `None` keeps them in memory.
    pub db_path: Option<PathBuf>,
    pub proxy: Proxy,
    pub quota: Option<Quota>,
    /// Where yt-dlp, ffmpeg and torrent state live. `None` uses a temporary folder.
    pub data_dir: Option<PathBuf>,
    /// Stop sharing a finished torrent at this upload-to-size ratio; 0 shares until stopped.
    pub seed_ratio: f64,
    /// Find torrent peers through the DHT (off only in tests).
    pub torrent_dht: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_active: 4,
            mode: SpeedMode::Balanced,
            limit: None,
            user_agent: concat!("KlyxDown/", env!("CARGO_PKG_VERSION")).into(),
            db_path: None,
            proxy: Proxy::System,
            quota: None,
            data_dir: None,
            seed_ratio: 2.0,
            torrent_dht: true,
        }
    }
}

// Events are rare (state changes only), so the size difference between variants doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Event {
    /// Added, or its state changed (not sent for plain progress; poll [`Engine::list`] for that).
    Changed(DownloadView),
    Removed(Id),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("That does not look like a link: {0}")]
    InvalidUrl(String),
    #[error("Could not reach the server: {0}")]
    Network(String),
    #[error("The server answered {0}")]
    Status(u16),
    #[error("File error: {0}")]
    Io(#[from] std::io::Error),
    #[error("That proxy address does not work: {0}")]
    InvalidProxy(String),
    #[error("No download with id {0}")]
    NotFound(Id),
    #[error("No queue with id {0}")]
    NoQueue(QueueId),
    #[error("That link is for a different file ({0}).")]
    DifferentFile(String),
    #[error("{0} is not installed yet.")]
    ToolMissing(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Queue settings arrive from the UI; keep them in range.
fn clean(mut q: Queue) -> Queue {
    q.max_active = q.max_active.clamp(1, 32);
    q.name = q.name.trim().chars().take(60).collect();
    q
}

/// Cheap to clone; all clones drive the same engine. Dropping the last clone stops every
/// download and saves where each one was.
#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
    _alive: Arc<Alive>,
}

/// Shuts the engine down when the last [`Engine`] clone goes away.
struct Alive(Arc<Shared>);

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

impl Engine {
    /// Loads the saved downloads (with `config.db_path`); interrupted ones resume by themselves.
    pub fn new(config: Config) -> Self {
        let shared = Arc::new(Shared::new(config));
        tokio::spawn(download::scheduler(
            Arc::downgrade(&shared),
            shared.wake.clone(),
        ));
        Self {
            _alive: Arc::new(Alive(shared.clone())),
            shared,
        }
    }

    pub async fn probe(&self, url: &str) -> Result<Probe> {
        Ok(probe::probe(&self.shared.client(), url, &[]).await?.0)
    }

    /// Like [`Engine::probe`], with the browser's cookies and referrer for logged-in downloads.
    pub async fn probe_with(&self, url: &str, headers: &[(String, String)]) -> Result<Probe> {
        Ok(probe::probe(&self.shared.client(), url, headers).await?.0)
    }

    /// Probes, creates the partial file and queues the download. Magnets and `.torrent`
    /// links become torrents; `media` makes it a video download; `convert` converts it after.
    pub async fn add(&self, mut req: AddRequest) -> Result<Id> {
        if req
            .convert
            .as_deref()
            .is_some_and(|p| !convert::is_preset(p))
        {
            req.convert = None;
        }
        if req.media.is_some() {
            return self.add_media(req).await;
        }
        if torrent::is_torrent_source(&req.url) || req.torrent_files.is_some() {
            return self.add_torrent(req).await;
        }
        let client = self.shared.client();
        let (mut probe, validators) = probe::probe(&client, &req.url, &req.headers).await?;
        let zip = match &req.zip_entry {
            Some(entry) => {
                let (entry, part) = zip::part(&client, &probe, &req.headers, entry).await?;
                probe.name = names::sanitize(entry.name.rsplit('/').next().unwrap_or_default());
                probe.size = Some(part.compressed);
                Some(part)
            }
            None => None,
        };
        req.mirrors = match zip {
            Some(_) => Vec::new(),
            None => {
                probe::matching_mirrors(&client, &probe, std::mem::take(&mut req.mirrors)).await
            }
        };
        let name = match req.file_name.as_deref().map(str::trim) {
            Some(n) if !n.is_empty() => names::sanitize(n),
            _ => probe.name.clone(),
        };
        let path = download::create_part(req.dir.clone(), name, probe.size).await?;
        let mut g = self.shared.lock();
        let id = g.next_id;
        g.next_id += 1;
        let dl = Download::new(id, req, probe, validators, path, zip);
        self.shared.emit(&dl);
        g.downloads.push(dl);
        drop(g);
        self.shared.wake.notify_one();
        Ok(id)
    }

    async fn add_media(&self, req: AddRequest) -> Result<Id> {
        let choice = req.media.clone().unwrap_or_default();
        probe::parse_url(&req.url)?;
        let title = req.file_name.clone().unwrap_or_else(|| "video".into());
        let extra = download::Extra {
            media: Some(media::MediaSpec {
                choice: choice.clone(),
                dir: req.dir.clone(),
            }),
            convert: req.convert.clone(),
            ..download::Extra::default()
        };
        let path = req.dir.join(names::sanitize(&title));
        Ok(self.push_job(|id| {
            Download::new_job(
                id,
                Kind::Media,
                req,
                names::sanitize(&title),
                path,
                choice.size,
                extra,
            )
        }))
    }

    async fn add_torrent(&self, req: AddRequest) -> Result<Id> {
        let torrents = self.shared.torrents();
        let info = torrents.inspect(&req.url).await?;
        let bytes = torrents.bytes_of(&info.info_hash).ok_or_else(|| {
            Error::Network("The torrent's details went missing; try again.".into())
        })?;
        if self
            .shared
            .lock()
            .downloads
            .iter()
            .any(|d| d.torrent_hash().as_deref() == Some(&info.info_hash))
        {
            return Err(Error::Network(
                "That torrent is already in the list.".into(),
            ));
        }
        let files = req.torrent_files.clone().filter(|f| !f.is_empty());
        let (dir, path) = torrent::target(&req.dir, &info, files.as_deref());
        let size = info
            .files
            .iter()
            .filter(|f| files.as_ref().is_none_or(|ids| ids.contains(&f.index)))
            .map(|f| f.size)
            .sum();
        let extra = download::Extra {
            torrent: Some(torrent::TorrentSpec {
                info_hash: info.info_hash.clone(),
                bytes: torrent::encode(&bytes),
                files,
                dir,
            }),
            seed: req.seed,
            convert: req.convert.clone(),
            ..download::Extra::default()
        };
        let name = req
            .file_name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(info.name.clone());
        Ok(self.push_job(|id| {
            Download::new_job(id, Kind::Torrent, req, name, path, Some(size), extra)
        }))
    }

    fn push_job(&self, make: impl FnOnce(Id) -> Download) -> Id {
        let mut g = self.shared.lock();
        let id = g.next_id;
        g.next_id += 1;
        let dl = make(id);
        self.shared.emit(&dl);
        g.downloads.push(dl);
        drop(g);
        self.shared.wake.notify_one();
        id
    }

    /// The qualities a web page's video offers (yt-dlp must be installed: see
    /// [`Engine::install_tools`]).
    pub async fn media_info(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<media::MediaInfo> {
        probe::parse_url(url)?;
        media::info(self.shared.tools(), url, headers).await
    }

    /// The files in a magnet, `.torrent` link or `.torrent` file on disk.
    pub async fn inspect_torrent(&self, source: &str) -> Result<torrent::TorrentInfo> {
        self.shared.torrents().inspect(source).await
    }

    pub async fn tools(&self) -> tools::ToolStatus {
        self.shared.tools().status().await
    }

    /// Downloads yt-dlp (and ffmpeg where KlyxDown provides it). Ask the user first.
    pub async fn install_tools(&self) -> Result<()> {
        self.shared.tools().install(&self.shared.client()).await
    }

    /// Converts a finished download with a preset; the conversion is a new job in the list.
    pub fn convert(&self, id: Id, preset: &str) -> Result<Id> {
        if !convert::is_preset(preset) {
            return Err(Error::Network(format!("Unknown conversion: {preset}")));
        }
        let (path, queue) = {
            let mut g = self.shared.lock();
            let dl = g.get_mut(id)?;
            if dl.state != State::Done {
                return Err(Error::Network(
                    "Only finished files can be converted.".into(),
                ));
            }
            (dl.path.clone(), dl.queue)
        };
        Ok(self.push_job(|new| convert::job(new, path, preset, queue)))
    }

    /// Starts or stops sharing a finished torrent.
    pub fn set_seeding(&self, id: Id, seed: bool) -> Result<()> {
        let mut g = self.shared.lock();
        let dl = g.get_mut(id)?;
        dl.set_seed(seed);
        self.shared.emit(dl);
        drop(g);
        self.shared.wake.notify_one();
        Ok(())
    }

    /// For the preview player: the file being written (partial or finished), its size, and
    /// how many bytes from `offset` on are already there.
    pub fn preview_at(&self, id: Id, offset: u64) -> Result<(PathBuf, Option<u64>, u64)> {
        let mut g = self.shared.lock();
        let dl = g.get_mut(id)?;
        let path = if dl.state == State::Done {
            dl.path.clone()
        } else {
            part_path(&dl.path)
        };
        Ok((path, dl.view_size(), dl.available_from(offset)))
    }

    /// The player needs `offset` next: download from there first.
    pub fn prioritize(&self, id: Id, offset: u64) -> Result<()> {
        let mut g = self.shared.lock();
        let dl = g.get_mut(id)?;
        if let Some(i) = dl.prioritize(offset) {
            self.shared.spawn_worker(dl, i);
        }
        Ok(())
    }

    /// A torrent's main file as a stream fetched in playing order (for the preview player):
    /// (file name, size, stream).
    pub async fn torrent_stream(&self, id: Id) -> Result<(String, u64, Box<dyn ReadSeek>)> {
        let (hash, files) = {
            let mut g = self.shared.lock();
            let dl = g.get_mut(id)?;
            let spec = dl.extra.torrent.clone().ok_or(Error::NotFound(id))?;
            (spec.info_hash, spec.files)
        };
        self.shared
            .torrents()
            .stream_main(&hash, files.as_deref())
            .await
    }

    pub fn pause(&self, id: Id) -> Result<()> {
        self.update(id, Download::pause)
    }

    pub fn resume(&self, id: Id) -> Result<()> {
        self.update(id, Download::resume)
    }

    /// Swaps in a fresh link for the same file (an expired or one-time link), keeping the
    /// progress. The new link must report the same size. Failed downloads queue up again.
    pub async fn refresh_link(
        &self,
        id: Id,
        url: &str,
        headers: Vec<(String, String)>,
    ) -> Result<()> {
        let (probe, validators) = probe::probe(&self.shared.client(), url, &headers).await?;
        let mut g = self.shared.lock();
        let dl = g.get_mut(id)?;
        dl.refresh(url, headers, probe, validators)?;
        self.shared.emit(dl);
        drop(g);
        self.shared.wake.notify_one();
        Ok(())
    }

    /// A small text resource (a Metalink file, a web page), at most `max` bytes.
    pub async fn fetch_small(&self, url: &str, max: u64) -> Result<Vec<u8>> {
        let url = probe::parse_url(url)?;
        let mut resp = self
            .shared
            .client()
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Network(probe::describe(&e)))?;
        if !resp.status().is_success() {
            return Err(Error::Status(resp.status().as_u16()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| Error::Network(probe::describe(&e)))?
        {
            if body.len() as u64 + chunk.len() as u64 > max {
                return Err(Error::Network("The page is too large.".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// What is inside a remote ZIP, read without downloading it (the server must support
    /// byte ranges).
    pub async fn list_zip(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<Vec<zip::ZipEntry>> {
        let client = self.shared.client();
        let (probe, _) = probe::probe(&client, url, headers).await?;
        zip::list(&client, &probe, headers).await
    }

    /// The files and folders in a folder: an FTP folder, or a web server's folder listing.
    /// Folders first.
    pub async fn list_folder(&self, url: &str) -> Result<Vec<ftp::Entry>> {
        let url = probe::parse_url(url)?;
        if ftp::is_ftp(url.as_str()) {
            ftp::list(&url).await
        } else {
            grab::explore(&self.shared.client(), &url).await
        }
    }

    /// Reads a site's pages from `url` and returns the links to files it finds (see
    /// [`grab::GrabOptions`]). Set `stop` to end early with what was found so far.
    pub async fn grab(
        &self,
        url: &str,
        opts: &grab::GrabOptions,
        stop: &std::sync::atomic::AtomicBool,
        progress: &(dyn Fn(grab::GrabProgress) + Send + Sync),
    ) -> Result<Vec<String>> {
        let url = probe::parse_url(url)?;
        grab::grab(&self.shared.client(), &url, opts, stop, progress).await
    }

    /// Throws the progress away and queues it from zero; the link is probed again on start.
    pub fn restart(&self, id: Id) -> Result<()> {
        self.update(id, Download::restart)
    }

    /// Stops every connection and saves the final progress. Call it when the app exits:
    /// apps often end with `process::exit`, which skips the save that dropping the last
    /// clone would do. Safe to call more than once.
    /// Switches the proxy for new connections; running ones finish on the old route.
    pub fn set_proxy(&self, proxy: Proxy) -> Result<()> {
        self.shared.set_proxy(&proxy)
    }

    pub fn shutdown(&self) {
        self.shared.shutdown();
    }

    /// Stops it and forgets it. `delete_file` also removes the partial or finished file.
    pub fn remove(&self, id: Id, delete_file: bool) -> Result<()> {
        let mut dl = self.shared.forget(id)?;
        dl.stop();
        let _ = self.shared.events.send(Event::Removed(id));
        self.shared.wake.notify_one();
        if let Some(hash) = dl.torrent_hash() {
            let shared = self.shared.clone();
            tokio::spawn(async move { shared.torrents().forget(&hash, delete_file).await });
            return Ok(());
        }
        for part in dl.temp_files() {
            let _ = std::fs::remove_file(part);
        }
        if delete_file && dl.kind() != Kind::Direct {
            if dl.state == State::Done {
                match std::fs::remove_file(&dl.path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
            return Ok(());
        }
        if delete_file {
            let file = if dl.state == State::Done {
                dl.path.clone()
            } else {
                part_path(&dl.path)
            };
            match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn pause_all(&self) {
        self.update_all(Download::pause);
    }

    pub fn resume_all(&self) {
        self.update_all(Download::resume);
    }

    pub fn set_mode(&self, mode: SpeedMode) {
        let mut g = self.shared.lock();
        g.mode = mode;
        let rate = effective_rate(g.mode, g.limit);
        self.shared.limiter.set_rate(rate);
        self.shared.torrents().set_rate(rate);
        drop(g);
        self.shared.wake.notify_one();
    }

    pub fn set_limit(&self, bytes_per_sec: Option<u64>) {
        let mut g = self.shared.lock();
        g.limit = bytes_per_sec;
        let rate = effective_rate(g.mode, g.limit);
        self.shared.limiter.set_rate(rate);
        self.shared.torrents().set_rate(rate);
    }

    /// Stop sharing finished torrents at this ratio (0: share until stopped by hand).
    pub fn set_seed_ratio(&self, ratio: f64) {
        self.shared.lock().seed_ratio = ratio.max(0.0);
    }

    /// Concurrency of the main queue. Lowering it sends the newest running downloads back to
    /// the queue (their progress stays).
    pub fn set_max_active(&self, n: usize) {
        let mut g = self.shared.lock();
        if let Some(q) = g.queue_mut(MAIN_QUEUE) {
            q.max_active = n.max(1);
            self.shared.db.save_queue(q.clone());
        }
        drop(g);
        self.shared.wake.notify_one();
    }

    pub fn queues(&self) -> Vec<Queue> {
        self.shared.lock().queues.clone()
    }

    /// Adds a queue (the id in `queue` is ignored) and returns its id.
    pub fn add_queue(&self, queue: Queue) -> QueueId {
        let mut g = self.shared.lock();
        let id = g.queues.iter().map(|q| q.id + 1).max().unwrap_or(1);
        let queue = Queue { id, ..clean(queue) };
        self.shared.db.save_queue(queue.clone());
        g.queues.push(queue);
        id
    }

    /// Renames, reschedules, stops or starts a queue.
    pub fn update_queue(&self, queue: Queue) -> Result<()> {
        let mut g = self.shared.lock();
        let q = g.queue_mut(queue.id).ok_or(Error::NoQueue(queue.id))?;
        *q = clean(queue);
        self.shared.db.save_queue(q.clone());
        drop(g);
        self.shared.wake.notify_one();
        Ok(())
    }

    /// Deletes a queue; its downloads move to the main queue. The main queue can't be removed.
    pub fn remove_queue(&self, id: QueueId) -> Result<()> {
        let mut g = self.shared.lock();
        let i = g
            .queues
            .iter()
            .position(|q| q.id == id && id != MAIN_QUEUE)
            .ok_or(Error::NoQueue(id))?;
        g.queues.remove(i);
        self.shared.db.delete_queue(id);
        for dl in g.downloads.iter_mut().filter(|d| d.queue == id) {
            dl.queue = MAIN_QUEUE;
            self.shared.emit(dl);
        }
        drop(g);
        self.shared.wake.notify_one();
        Ok(())
    }

    pub fn move_to_queue(&self, id: Id, queue: QueueId) -> Result<()> {
        let mut g = self.shared.lock();
        if !g.queues.iter().any(|q| q.id == queue) {
            return Err(Error::NoQueue(queue));
        }
        let dl = g.get_mut(id)?;
        dl.queue = queue;
        self.shared.emit(dl);
        drop(g);
        self.shared.wake.notify_one();
        Ok(())
    }

    /// Holds every download (they wait as `Queued`, progress kept) while `metered` is true.
    pub fn set_metered_hold(&self, metered: bool) {
        self.shared.lock().metered = metered;
        self.shared.wake.notify_one();
    }

    pub fn set_quota(&self, quota: Option<Quota>) {
        let mut g = self.shared.lock();
        g.quota.set(quota.filter(|q| q.bytes > 0 && q.hours > 0));
        drop(g);
        self.shared.wake.notify_one();
    }

    pub fn status(&self) -> Status {
        let g = self.shared.lock();
        Status {
            hold: g.hold(),
            quota_used: g.quota.used(),
        }
    }

    /// The UI reports OS network changes. Offline: no new connection attempts.
    /// Online: everything in `WaitingForNetwork` retries right away.
    pub fn set_online(&self, online: bool) {
        tracing::debug!(online, "network state");
        self.shared.online.send_replace(online);
    }

    pub fn list(&self) -> Vec<DownloadView> {
        self.shared
            .lock()
            .downloads
            .iter()
            .map(Download::view)
            .collect()
    }

    pub fn total_speed(&self) -> u64 {
        self.shared
            .lock()
            .downloads
            .iter()
            .map(Download::speed)
            .sum()
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    /// Scales every retry delay (normally 1 s, doubling up to 30 s). For tests.
    #[doc(hidden)]
    pub fn set_retry_base_for_tests(&self, ms: u64) {
        self.shared.retry_base_ms.store(ms, Ordering::Relaxed);
    }

    /// Shortens the slow-connection rule (normally 5 s grace, 10 s slow, 30 s between
    /// replacements). For tests.
    #[doc(hidden)]
    pub fn set_slow_rule_for_tests(&self, grace_ms: u64, window_ms: u64, cooldown_ms: u64) {
        self.shared.lock().slow = SlowRule {
            grace: Duration::from_millis(grace_ms),
            window: Duration::from_millis(window_ms),
            cooldown: Duration::from_millis(cooldown_ms),
        };
    }

    /// Torrent peers to connect to right away (a local seeder). For tests.
    #[doc(hidden)]
    pub fn set_torrent_peers_for_tests(&self, peers: Vec<std::net::SocketAddr>) {
        *self
            .shared
            .torrents()
            .peers
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = peers;
    }

    /// Slow connections replaced so far. For tests.
    #[doc(hidden)]
    pub fn replacements_for_tests(&self) -> u64 {
        self.shared.replaced.load(Ordering::Relaxed)
    }

    fn update(&self, id: Id, change: fn(&mut Download) -> bool) -> Result<()> {
        let mut g = self.shared.lock();
        let dl = g.get_mut(id)?;
        if change(dl) {
            self.shared.emit(dl);
        }
        drop(g);
        self.shared.wake.notify_one();
        Ok(())
    }

    fn update_all(&self, change: fn(&mut Download) -> bool) {
        let mut g = self.shared.lock();
        for dl in g
            .downloads
            .iter_mut()
            .filter_map(|d| change(d).then_some(d))
        {
            self.shared.emit(dl);
        }
        drop(g);
        self.shared.wake.notify_one();
    }
}
