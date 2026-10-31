//! KlyxDown download engine.
//!
//! Pure Rust, no Tauri. The app talks to it through [`Engine`]: commands in,
//! [`DownloadView`] snapshots and [`Event`]s out.
//!
//! Must be created inside a Tokio runtime ([`Engine::new`] spawns its scheduler).

mod db;
mod download;
mod limiter;
mod names;
mod probe;
mod queue;
mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use download::{Download, Shared, SlowRule, effective_rate, part_path};

pub use queue::{MAIN_QUEUE, Queue, QueueId, Schedule};

pub type Id = u64;

/// How a download gets its bytes.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A link to a file (HTTP, HTTPS, FTP), fetched in segments.
    #[default]
    Direct,
}

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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum State {
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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
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

    /// Probes, creates the partial file and queues the download.
    pub async fn add(&self, req: AddRequest) -> Result<Id> {
        let (probe, validators) =
            probe::probe(&self.shared.client(), &req.url, &req.headers).await?;
        let name = match req.file_name.as_deref().map(str::trim) {
            Some(n) if !n.is_empty() => names::sanitize(n),
            _ => probe.name.clone(),
        };
        let path = download::create_part(req.dir.clone(), name, probe.size).await?;
        let mut g = self.shared.lock();
        let id = g.next_id;
        g.next_id += 1;
        let dl = Download::new(id, req, probe, validators, path);
        self.shared.emit(&dl);
        g.downloads.push(dl);
        drop(g);
        self.shared.wake.notify_one();
        Ok(id)
    }

    pub fn pause(&self, id: Id) -> Result<()> {
        self.update(id, Download::pause)
    }

    pub fn resume(&self, id: Id) -> Result<()> {
        self.update(id, Download::resume)
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
        self.shared
            .limiter
            .set_rate(effective_rate(g.mode, g.limit));
        drop(g);
        self.shared.wake.notify_one();
    }

    pub fn set_limit(&self, bytes_per_sec: Option<u64>) {
        let mut g = self.shared.lock();
        g.limit = bytes_per_sec;
        self.shared
            .limiter
            .set_rate(effective_rate(g.mode, g.limit));
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
