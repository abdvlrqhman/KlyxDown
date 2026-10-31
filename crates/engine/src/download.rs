//! Engine state, the scheduler task and the segment bookkeeping (start, split, replace, finish).

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{Notify, broadcast, watch};
use tokio::task::AbortHandle;
use tracing::{debug, warn};

use serde::{Deserialize, Serialize};

use crate::checksum::{self, Checksum};
use crate::db::{Db, Record};
use crate::probe::{self, Validators};
use crate::queue::{MAIN_QUEUE, Queue, QueueId, local_now};
use crate::runner::{self, Ext};
use crate::tools::Tools;
use crate::torrent::Torrents;
use crate::worker::{self, Job, Stop};
use crate::zip::{self, ZipPart};
use crate::{
    AddRequest, Config, DownloadView, Error, Event, Hold, Id, Kind, Probe, Quota, Result,
    SegmentView, SpeedMode, State,
};
use crate::{Proxy, limiter::Limiter, names};
use crate::{convert, media, torrent};

/// A running segment is split only when more than this is left, so both halves stay above 1 MiB.
const SPLIT_MIN: u64 = 2 * 1024 * 1024;
/// Initial segments are at least this big; tiny files don't get 32 connections.
const MIN_SEGMENT: u64 = 1024 * 1024;
const SNAIL_RATE: u64 = 512 * 1024;
const CHECKSUM_MISMATCH: &str = "The file does not match its checksum, so it may be damaged or \
                                 changed. Restart it to download it again.";
const TICK: Duration = Duration::from_millis(250);
/// Progress is checkpointed every this many ticks (once a second).
const CHECKPOINT_TICKS: u32 = 4;
/// Speed smoothing time constant, in seconds.
const SPEED_TAU: f64 = 1.0;

pub(crate) struct Shared {
    inner: Mutex<Inner>,
    pub(crate) events: broadcast::Sender<Event>,
    /// Rebuilt when the proxy changes; read it through [`Shared::client`].
    client: RwLock<reqwest::Client>,
    user_agent: String,
    pub(crate) limiter: Limiter,
    pub(crate) wake: Arc<Notify>,
    pub(crate) online: watch::Sender<bool>,
    pub(crate) retry_base_ms: AtomicU64,
    /// Slow connections replaced so far.
    pub(crate) replaced: AtomicU64,
    /// Only written while `inner` is locked, so the queue order matches the state order.
    pub(crate) db: Db,
    tools: Tools,
    torrents: Torrents,
}

pub(crate) struct Inner {
    pub(crate) next_id: Id,
    /// Never empty: the main queue is always first.
    pub(crate) queues: Vec<Queue>,
    /// The connection is metered and the user wants downloads held on it.
    pub(crate) metered: bool,
    pub(crate) quota: QuotaMeter,
    pub(crate) mode: SpeedMode,
    pub(crate) limit: Option<u64>,
    pub(crate) downloads: Vec<Download>,
    pub(crate) slow: SlowRule,
    /// The last `Engine` handle is gone; nothing starts anymore.
    closed: bool,
    /// Stop sharing a torrent at this upload-to-size ratio; 0 never stops.
    pub(crate) seed_ratio: f64,
}

/// Counts downloaded bytes against an optional "N bytes every H hours" quota.
#[derive(Default)]
pub(crate) struct QuotaMeter {
    quota: Option<Quota>,
    used: u64,
    since: Option<Instant>,
}

impl QuotaMeter {
    // shortcut: the period restarts with the app, so a restart forgets what was used;
    // persist `used` and the period start if people rely on it for hard data caps.
    pub(crate) fn set(&mut self, quota: Option<Quota>) {
        if quota != self.quota {
            *self = Self {
                quota,
                ..Self::default()
            };
        }
    }

    pub(crate) fn used(&self) -> u64 {
        self.quota.map_or(0, |_| self.used)
    }

    fn period(&self) -> Option<Duration> {
        self.quota
            .map(|q| Duration::from_secs(u64::from(q.hours) * 3600))
    }

    fn add(&mut self, bytes: u64, now: Instant) {
        let Some(period) = self.period() else { return };
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) >= period {
            self.since = Some(now);
            self.used = 0;
        }
        self.used += bytes;
    }

    /// When the used-up quota frees again.
    fn blocked_until(&self, now: Instant) -> Option<Instant> {
        let (quota, since, period) = (self.quota?, self.since?, self.period()?);
        let until = since + period;
        (self.used >= quota.bytes && now < until).then_some(until)
    }
}

/// When a connection is slow enough to be replaced by a fresh one.
#[derive(Clone, Copy)]
pub(crate) struct SlowRule {
    /// A new connection is left alone this long.
    pub(crate) grace: Duration,
    /// Under a quarter of the median speed for this long.
    pub(crate) window: Duration,
    /// At most one replacement per segment per this long.
    pub(crate) cooldown: Duration,
}

impl Default for SlowRule {
    fn default() -> Self {
        Self {
            grace: Duration::from_secs(5),
            window: Duration::from_secs(10),
            cooldown: Duration::from_secs(30),
        }
    }
}

pub(crate) struct Download {
    id: Id,
    kind: Kind,
    pub(crate) queue: QueueId,
    pub(crate) extra: Extra,
    /// As the user gave it; shown in the UI.
    url: String,
    /// After redirects; what the workers fetch.
    fetch_url: String,
    name: String,
    host: String,
    pub(crate) path: PathBuf,
    size: Option<u64>,
    resumable: bool,
    connections: u8,
    headers: Vec<(String, String)>,
    validators: Validators,
    pub(crate) state: State,
    error: Option<String>,
    added_at: u64,
    finished_at: Option<u64>,
    /// Empty means the progress was thrown away: probe again and recreate the partial file.
    segments: Vec<Segment>,
    /// Bumped whenever workers are stopped, so late reports from old workers are ignored.
    run: u64,
    /// Last worker token handed out.
    tokens: u64,
    probing: bool,
    finishing: bool,
    /// The re-probe and finish tasks; workers live in their segment.
    tasks: Vec<AbortHandle>,
    /// Torrents, videos and conversions: the one task doing the work, and what it reports.
    runner: Option<AbortHandle>,
    pub(crate) ext: Arc<Ext>,
    ext_last: (u64, u64),
    ext_speed: (f64, f64),
}

/// Per-download details that only some downloads have, stored as JSON.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Extra {
    /// The server refused the link; a fresh one from the page would fix it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) needs_link: bool,
    /// Same-file links (after redirects) that share the segments with `fetch_url`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) mirrors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checksum: Option<Checksum>,
    /// One file from a remote ZIP: the download holds its packed bytes until it completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) zip: Option<ZipPart>,
    /// Progress of a torrent, video or conversion when it was last saved.
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) done: u64,
    /// A conversion preset to run on the file once it is complete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) convert: Option<String>,
    /// Video or audio from a web page, through yt-dlp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) media: Option<media::MediaSpec>,
    /// A conversion job: what to convert, and how.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) job: Option<convert::ConvertSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) torrent: Option<torrent::TorrentSpec>,
    /// Keep sharing a finished torrent.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) seed: bool,
    /// Temporary files a runner writes (yt-dlp `.part` files), removed with the download.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) parts: Vec<PathBuf>,
    /// Bytes a torrent shared before the last restart.
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) uploaded: u64,
}

fn ratio_left(ratio: f64, dl: &Download) -> bool {
    ratio <= 0.0 || (dl.ext.sent.load(Relaxed) as f64) < ratio * dl.size.unwrap_or(0) as f64
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

struct Segment {
    start: u64,
    prog: Arc<Progress>,
    running: bool,
    retrying: bool,
    /// Identifies the current worker, so a replaced one's late reports are ignored.
    token: u64,
    task: Option<AbortHandle>,
    started: Instant,
    slow_since: Option<Instant>,
    replaced_at: Option<Instant>,
    last_recv: u64,
    speed: f64,
}

/// The part of a segment its worker updates without the lock.
pub(crate) struct Progress {
    /// Exclusive; `u64::MAX` while the size is unknown. Lowered by a split, the worker re-reads it.
    pub(crate) end: AtomicU64,
    /// Next byte to write; everything before it is on disk.
    pub(crate) pos: AtomicU64,
    /// Every byte received, including discarded ones; feeds the speed.
    pub(crate) recv: AtomicU64,
}

impl Segment {
    fn new(start: u64, end: u64, pos: u64) -> Self {
        let prog = Progress {
            end: AtomicU64::new(end),
            pos: AtomicU64::new(pos),
            recv: AtomicU64::new(0),
        };
        Self {
            start,
            prog: Arc::new(prog),
            running: false,
            retrying: false,
            token: 0,
            task: None,
            started: Instant::now(),
            slow_since: None,
            replaced_at: None,
            last_recv: 0,
            speed: 0.0,
        }
    }

    fn end(&self) -> u64 {
        self.prog.end.load(Relaxed)
    }

    fn pos(&self) -> u64 {
        self.prog.pos.load(Relaxed).min(self.end())
    }

    fn done(&self) -> bool {
        self.pos() >= self.end()
    }

    fn view(&self, active: bool) -> SegmentView {
        let (pos, end) = (self.pos(), self.end());
        SegmentView {
            start: self.start,
            // Unknown size: the segment ends where the data does so far.
            end: if end == u64::MAX { pos } else { end },
            pos,
            speed: if active { self.speed.round() as u64 } else { 0 },
        }
    }
}

impl Download {
    pub(crate) fn new(
        id: Id,
        req: AddRequest,
        probe: Probe,
        validators: Validators,
        path: PathBuf,
        zip: Option<ZipPart>,
    ) -> Self {
        let connections = if probe.resumable && probe.size.is_some() {
            req.connections.clamp(1, 32)
        } else {
            1
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(probe.name);
        Self {
            id,
            kind: Kind::Direct,
            queue: req.queue,
            extra: Extra {
                mirrors: req.mirrors,
                checksum: req.checksum,
                zip,
                ..Extra::default()
            },
            url: req.url,
            fetch_url: probe.url,
            name,
            host: probe.host,
            path,
            size: probe.size,
            resumable: probe.resumable,
            connections,
            headers: req.headers,
            validators,
            state: if req.start {
                State::Queued
            } else {
                State::Paused
            },
            error: None,
            added_at: now_ms(),
            finished_at: None,
            segments: initial_segments(probe.size, connections),
            run: 0,
            tokens: 0,
            probing: false,
            finishing: false,
            tasks: Vec::new(),
            runner: None,
            ext: Arc::default(),
            ext_last: (0, 0),
            ext_speed: (0.0, 0.0),
        }
    }

    /// A torrent, video or conversion: no segments, a runner task does the work.
    pub(crate) fn new_job(
        id: Id,
        kind: Kind,
        req: AddRequest,
        name: String,
        path: PathBuf,
        size: Option<u64>,
        extra: Extra,
    ) -> Self {
        // Web pages show their site; magnets and local files have none.
        let host = reqwest::Url::parse(&req.url)
            .ok()
            .filter(|u| u.scheme().starts_with("http"))
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        Self {
            id,
            kind,
            queue: req.queue,
            extra,
            url: req.url.clone(),
            fetch_url: req.url,
            name,
            host,
            path,
            size,
            resumable: true,
            connections: 1,
            headers: req.headers,
            validators: Validators::default(),
            state: if req.start {
                State::Queued
            } else {
                State::Paused
            },
            error: None,
            added_at: now_ms(),
            finished_at: None,
            segments: Vec::new(),
            run: 0,
            tokens: 0,
            probing: false,
            finishing: false,
            tasks: Vec::new(),
            runner: None,
            ext: Arc::default(),
            ext_last: (0, 0),
            ext_speed: (0.0, 0.0),
        }
    }

    /// Back from the database. Interrupted downloads queue up again; a vanished partial file
    /// starts over from zero.
    fn restore(r: Record) -> Self {
        let state = match r.state {
            State::Downloading | State::Queued | State::WaitingForNetwork => State::Queued,
            other => other,
        };
        let mut dl = Self {
            id: r.id,
            kind: r.kind,
            queue: r.queue,
            extra: r.extra,
            url: r.url,
            fetch_url: r.final_url,
            name: r.name,
            host: r.host,
            path: r.path,
            size: r.size,
            resumable: r.resumable,
            connections: r.connections.clamp(1, 32),
            headers: r.headers,
            validators: Validators {
                etag: r.etag,
                last_modified: r.last_modified,
            },
            error: if state == State::Queued {
                None
            } else {
                r.error
            },
            state,
            added_at: r.added_at,
            finished_at: r.finished_at,
            segments: r
                .segments
                .into_iter()
                .map(|(start, end, pos)| Segment::new(start, end, pos))
                .collect(),
            run: 0,
            tokens: 0,
            probing: false,
            finishing: false,
            tasks: Vec::new(),
            runner: None,
            ext: Arc::default(),
            ext_last: (0, 0),
            ext_speed: (0.0, 0.0),
        };
        dl.ext.done.store(dl.extra.done, Relaxed);
        dl.ext.sent.store(dl.extra.uploaded, Relaxed);
        dl.ext.total.store(dl.size.unwrap_or(0), Relaxed);
        let part = part_path(&dl.path);
        if dl.kind == Kind::Direct
            && dl.state != State::Done
            && !dl.segments.is_empty()
            && !part.exists()
        {
            warn!(id = dl.id, path = %part.display(), "partial file missing, starting over");
            dl.segments = initial_segments(dl.size, dl.connections);
            // An error here resurfaces as `Failed` when a worker opens the file.
            if let Err(e) = create_part_file(&part, dl.size) {
                warn!(id = dl.id, "could not recreate the partial file: {e}");
            }
        }
        dl
    }

    fn record(&self) -> Record {
        Record {
            id: self.id,
            url: self.url.clone(),
            final_url: self.fetch_url.clone(),
            name: self.name.clone(),
            host: self.host.clone(),
            path: self.path.clone(),
            size: self.size,
            state: self.state,
            error: self.error.clone(),
            resumable: self.resumable,
            connections: self.connections,
            headers: self.headers.clone(),
            etag: self.validators.etag.clone(),
            last_modified: self.validators.last_modified.clone(),
            added_at: self.added_at,
            finished_at: self.finished_at,
            segments: self
                .segments
                .iter()
                .map(|s| (s.start, s.end(), s.pos()))
                .collect(),
            kind: self.kind,
            queue: self.queue,
            extra: Extra {
                done: self.ext.done.load(Relaxed),
                uploaded: self.ext.sent.load(Relaxed).max(self.extra.uploaded),
                parts: {
                    let mut parts = self.extra.parts.clone();
                    for p in self
                        .ext
                        .parts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter()
                    {
                        if !parts.contains(p) {
                            parts.push(p.clone());
                        }
                    }
                    parts
                },
                ..self.extra.clone()
            },
        }
    }

    fn is_active(&self) -> bool {
        matches!(self.state, State::Downloading | State::WaitingForNetwork)
    }

    fn running(&self) -> usize {
        self.segments.iter().filter(|s| s.running).count()
    }

    fn bytes(&self) -> u64 {
        if self.kind != Kind::Direct {
            return self.ext.done.load(Relaxed);
        }
        self.segments.iter().map(|s| s.pos() - s.start).sum()
    }

    fn total(&self) -> Option<u64> {
        match self.ext.total.load(Relaxed) {
            0 => self.size,
            n if self.kind != Kind::Direct => Some(n),
            _ => self.size,
        }
    }

    pub(crate) fn speed(&self) -> u64 {
        if !self.is_active() {
            0
        } else if self.kind != Kind::Direct {
            self.ext_speed.0.round() as u64
        } else {
            self.segments.iter().map(|s| s.speed.round() as u64).sum()
        }
    }

    pub(crate) fn view_size(&self) -> Option<u64> {
        self.total()
    }

    /// A finished torrent still sharing with other people.
    fn seeding(&self) -> bool {
        self.kind == Kind::Torrent && self.state == State::Done && self.runner.is_some()
    }

    pub(crate) fn view(&self) -> DownloadView {
        let active = self.is_active();
        DownloadView {
            id: self.id,
            url: self.url.clone(),
            name: self.name.clone(),
            host: self.host.clone(),
            path: self.path.clone(),
            size: self.total(),
            bytes: self.bytes(),
            speed: self.speed(),
            state: self.state,
            error: self.error.clone(),
            resumable: self.resumable,
            connections: self.connections,
            segments: if self.kind == Kind::Direct {
                self.segments.iter().map(|s| s.view(active)).collect()
            } else {
                // One lane for the whole job, so the file map still fills up.
                let (pos, end) = (self.bytes(), self.total().unwrap_or(0));
                vec![SegmentView {
                    start: 0,
                    end: end.max(pos),
                    pos,
                    speed: self.speed(),
                }]
            },
            detail: self.ext.detail(),
            upload_speed: if self.runner.is_some() {
                self.ext_speed.1.round() as u64
            } else {
                0
            },
            uploaded: self.ext.sent.load(Relaxed),
            peers: self.ext.peers.load(Relaxed) as u32,
            seeding: self.seeding(),
            added_at: self.added_at,
            finished_at: self.finished_at,
            kind: self.kind,
            queue: self.queue,
            needs_link: self.extra.needs_link,
            mirrors: self.extra.mirrors.len(),
            page: self
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("referer"))
                .map(|(_, v)| v.clone()),
        }
    }

    /// A fresh link for the same file. Keeps the progress; a failed download queues again.
    pub(crate) fn refresh(
        &mut self,
        url: &str,
        headers: Vec<(String, String)>,
        probe: Probe,
        validators: Validators,
    ) -> Result<()> {
        if let (Some(have), Some(new)) = (self.size, probe.size)
            && have != new
        {
            return Err(Error::DifferentFile(format!("{new} bytes, not {have}")));
        }
        if self.state == State::Done {
            return Ok(());
        }
        let running = self.is_active();
        self.stop();
        self.url = url.to_string();
        self.fetch_url = probe.url;
        if !headers.is_empty() {
            self.headers = headers;
        }
        self.validators = validators;
        self.resumable = probe.resumable;
        self.extra.needs_link = false;
        if running || self.state == State::Failed {
            self.state = State::Queued;
        }
        self.error = None;
        Ok(())
    }

    /// The segment `job` works on, if `job` is still its worker.
    fn worker_segment(&mut self, job: &Job) -> Option<&mut Segment> {
        self.segments
            .iter_mut()
            .find(|s| s.start == job.start && s.token == job.token)
    }

    /// Aborts every task of the current run. Progress already on disk is kept.
    pub(crate) fn stop(&mut self) {
        self.tasks.drain(..).for_each(|t| t.abort());
        // Dropping a runner kills its yt-dlp or ffmpeg, or pauses its torrent.
        if let Some(t) = self.runner.take() {
            t.abort();
        }
        for s in &mut self.segments {
            if let Some(t) = s.task.take() {
                t.abort();
            }
            s.running = false;
            s.retrying = false;
        }
        self.run += 1;
        self.probing = false;
        self.finishing = false;
    }

    fn fail(&mut self, msg: String) {
        self.fail_with(Stop {
            msg,
            expired: false,
        });
    }

    fn fail_with(&mut self, stop: Stop) {
        let msg = stop.msg;
        self.extra.needs_link = stop.expired;
        warn!(id = self.id, "download failed: {msg}");
        self.stop();
        self.state = State::Failed;
        self.error = Some(msg);
    }

    /// Returns whether the state changed.
    pub(crate) fn pause(&mut self) -> bool {
        // Once the rename is under way there is nothing left to pause.
        if !matches!(
            self.state,
            State::Queued | State::Downloading | State::WaitingForNetwork
        ) || self.finishing
        {
            return false;
        }
        self.stop();
        self.state = State::Paused;
        self.error = None;
        true
    }

    /// Returns whether the state changed.
    pub(crate) fn resume(&mut self) -> bool {
        if !matches!(self.state, State::Paused | State::Failed) {
            return false;
        }
        self.state = State::Queued;
        self.error = None;
        true
    }

    pub(crate) fn kind(&self) -> Kind {
        self.kind
    }

    pub(crate) fn torrent_hash(&self) -> Option<String> {
        self.extra.torrent.as_ref().map(|t| t.info_hash.clone())
    }

    /// Temporary files of an unfinished video or conversion.
    pub(crate) fn temp_files(&self) -> Vec<PathBuf> {
        if self.state == State::Done {
            return Vec::new();
        }
        let mut files = self.extra.parts.clone();
        files.extend(
            self.ext
                .parts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned(),
        );
        files
    }

    /// Sharing a finished torrent: off stops it right away, on starts it again.
    pub(crate) fn set_seed(&mut self, seed: bool) {
        self.extra.seed = seed;
        if !seed && self.state == State::Done {
            self.stop();
        }
    }

    /// Bytes on disk from `offset` on without a gap (for the preview player).
    pub(crate) fn available_from(&self, offset: u64) -> u64 {
        if self.state == State::Done {
            return self.size.unwrap_or(0).saturating_sub(offset);
        }
        let mut at = offset;
        for s in &self.segments {
            if s.end() <= at || s.start > at {
                continue;
            }
            if s.pos() <= at {
                break;
            }
            at = s.pos();
            if !s.done() {
                break;
            }
        }
        at - offset
    }

    /// Starts a connection at `offset` (the player jumped there) unless one is about to
    /// reach it. Returns the segment to start, if a new one was made.
    pub(crate) fn prioritize(&mut self, offset: u64) -> Option<usize> {
        const NEAR: u64 = 2 * 1024 * 1024;
        if self.kind != Kind::Direct
            || !self.resumable
            || self.size.is_none()
            || self.state != State::Downloading
        {
            return None;
        }
        let i = self
            .segments
            .iter()
            .position(|s| s.start <= offset && offset < s.end())?;
        let s = &self.segments[i];
        let (pos, end) = (s.pos(), s.end());
        if offset < pos || (s.running && offset - pos <= NEAR) || end - offset < MIN_SEGMENT {
            return None;
        }
        s.prog.end.store(offset, Relaxed);
        debug!(
            id = self.id,
            offset, "preview jumped ahead, new connection there"
        );
        self.segments
            .insert(i + 1, Segment::new(offset, end, offset));
        Some(i + 1)
    }

    /// What to convert once this download is complete, taken so it happens once.
    fn convert_after(&mut self) -> Option<(PathBuf, String, QueueId)> {
        let preset = self.extra.convert.take()?;
        Some((self.path.clone(), preset, self.queue))
    }

    /// Back to the queue with its progress kept: its queue closed or has too many running.
    fn requeue(&mut self) -> bool {
        if !self.is_active() || self.finishing {
            return false;
        }
        self.stop();
        self.state = State::Queued;
        self.error = None;
        true
    }

    /// Throws the progress away and queues it; the link is probed again when it starts.
    pub(crate) fn restart(&mut self) -> bool {
        self.stop();
        self.segments.clear();
        self.ext.done.store(0, Relaxed);
        self.state = State::Queued;
        self.error = None;
        self.finished_at = None;
        true
    }

    fn begin(&mut self) {
        debug!(id = self.id, name = %self.name, "starting");
        self.state = State::Downloading;
        self.error = None;
        if !self.resumable && !self.segments.is_empty() {
            // Without ranges the only way to continue is from the start.
            self.segments = vec![Segment::new(0, self.size.unwrap_or(u64::MAX), 0)];
        }
    }

    /// Splits the running segment with the most bytes left at its midpoint (the IDM trick).
    /// Returns the index of the new segment.
    fn split(&mut self) -> Option<usize> {
        if !self.resumable || self.size.is_none() {
            return None;
        }
        let (i, seg) = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.running)
            .max_by_key(|(_, s)| s.end() - s.pos())?;
        let (pos, end) = (seg.pos(), seg.end());
        if end - pos <= SPLIT_MIN {
            return None;
        }
        let mid = pos + (end - pos) / 2;
        seg.prog.end.store(mid, Relaxed);
        debug!(id = self.id, from = seg.start, mid, end, "split segment");
        self.segments.insert(i + 1, Segment::new(mid, end, mid));
        Some(i + 1)
    }
}

fn initial_segments(size: Option<u64>, connections: u8) -> Vec<Segment> {
    let Some(size) = size else {
        return vec![Segment::new(0, u64::MAX, 0)];
    };
    let n = u64::from(connections)
        .min(size.div_ceil(MIN_SEGMENT))
        .max(1);
    (0..n)
        .map(|i| (size * i / n, size * (i + 1) / n))
        .map(|(start, end)| Segment::new(start, end, start))
        .collect()
}

pub(crate) fn part_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".kdpart");
    p.into()
}

/// Creates or empties the partial file, preallocated when the size is known.
fn create_part_file(part: &Path, size: Option<u64>) -> io::Result<()> {
    if let Some(dir) = part.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(part)?;
    if let Some(len) = size {
        file.set_len(len)?;
    }
    Ok(())
}

/// Picks `name`, `name (1)`, ... in `dir` (skipping names whose `.kdpart` exists) and creates
/// the partial file, preallocated when the size is known.
pub(crate) async fn create_part(dir: PathBuf, name: String, size: Option<u64>) -> Result<PathBuf> {
    let made = tokio::task::spawn_blocking(move || -> io::Result<PathBuf> {
        std::fs::create_dir_all(&dir)?;
        for n in 0..10_000 {
            let path = dir.join(names::numbered(&name, n));
            if path.exists() {
                continue;
            }
            let part = part_path(&path);
            match OpenOptions::new().write(true).create_new(true).open(&part) {
                Ok(file) => {
                    if let Some(len) = size
                        && let Err(e) = file.set_len(len)
                    {
                        drop(file);
                        let _ = std::fs::remove_file(&part);
                        return Err(e);
                    }
                    return Ok(path);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free file name",
        ))
    })
    .await;
    Ok(made.map_err(io::Error::other)??)
}

/// Windows "Mark of the Web": Explorer, SmartScreen and Office treat the file as downloaded.
/// Errors are ignored; FAT32/exFAT drives have no alternate streams.
#[cfg(windows)]
fn mark_of_the_web(path: &Path, referrer: Option<&str>, host_url: &str) {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    let mut text = String::from("[ZoneTransfer]\r\nZoneId=3\r\n");
    if let Some(r) = referrer {
        text.push_str(&format!("ReferrerUrl={r}\r\n"));
    }
    text.push_str(&format!("HostUrl={host_url}\r\n"));
    if let Err(e) = std::fs::write(stream, text) {
        debug!(path = %path.display(), "no Zone.Identifier stream: {e}");
    }
}

#[cfg(not(windows))]
fn mark_of_the_web(_: &Path, _: Option<&str>, _: &str) {}

pub(crate) fn effective_rate(mode: SpeedMode, limit: Option<u64>) -> Option<u64> {
    let limit = limit.filter(|&l| l > 0);
    match mode {
        SpeedMode::Snail => Some(limit.map_or(SNAIL_RATE, |l| l.min(SNAIL_RATE))),
        SpeedMode::Balanced | SpeedMode::Full => limit,
    }
}

fn mode_connections(mode: SpeedMode) -> u8 {
    match mode {
        SpeedMode::Snail => 2,
        SpeedMode::Balanced => 4,
        SpeedMode::Full => u8::MAX,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Inner {
    pub(crate) fn queue_mut(&mut self, id: QueueId) -> Option<&mut Queue> {
        self.queues.iter_mut().find(|q| q.id == id)
    }

    pub(crate) fn hold(&self) -> Option<Hold> {
        if self.metered {
            return Some(Hold::Metered);
        }
        let now = Instant::now();
        let until = self.quota.blocked_until(now)?;
        Some(Hold::Quota {
            resumes_at: now_ms() + until.duration_since(now).as_millis() as u64,
        })
    }

    /// Which queues may run now, and how many downloads each may run.
    fn allowance(&self) -> Vec<(QueueId, usize)> {
        let held = self.hold().is_some();
        let (weekday, minute) = local_now();
        self.queues
            .iter()
            .map(|q| {
                let open =
                    !held && !q.stopped && q.schedule.is_none_or(|s| s.is_open(weekday, minute));
                (q.id, if open { q.max_active } else { 0 })
            })
            .collect()
    }

    /// A finished torrent still has sharing to do.
    fn seed_ratio_left(&self, dl: &Download) -> bool {
        ratio_left(self.seed_ratio, dl)
    }

    pub(crate) fn get_mut(&mut self, id: Id) -> Result<&mut Download> {
        self.downloads
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or(Error::NotFound(id))
    }

    /// The download, only if `run` is still its current run.
    fn current(&mut self, id: Id, run: u64) -> Option<&mut Download> {
        self.downloads
            .iter_mut()
            .find(|d| d.id == id && d.run == run)
    }
}

impl Shared {
    pub(crate) fn new(config: Config) -> Self {
        let client = build_client(&config.user_agent, &config.proxy).unwrap_or_else(|e| {
            warn!("proxy setting ignored: {e}");
            build_client(&config.user_agent, &Proxy::System)
                .expect("HTTP client (TLS backend or user agent) failed to initialize")
        });
        let (db, records, mut queues) = Db::open(config.db_path.as_deref());
        let mut downloads: Vec<Download> = records.into_iter().map(Download::restore).collect();
        // The main queue always exists, first; its concurrency comes from the settings.
        queues.retain(|q| q.id != MAIN_QUEUE);
        queues.insert(
            0,
            Queue {
                id: MAIN_QUEUE,
                name: "Main".into(),
                max_active: config.max_active.max(1),
                stopped: false,
                schedule: None,
            },
        );
        db.save_queue(queues[0].clone());
        for dl in downloads
            .iter_mut()
            .filter(|d| !queues.iter().any(|q| q.id == d.queue))
        {
            dl.queue = MAIN_QUEUE;
        }
        let mut quota = QuotaMeter::default();
        quota.set(config.quota);
        let data_dir = config
            .data_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("klyxdown"));
        Self {
            limiter: Limiter::new(effective_rate(config.mode, config.limit)),
            inner: Mutex::new(Inner {
                next_id: downloads.iter().map(|d| d.id + 1).max().unwrap_or(1),
                queues,
                metered: false,
                quota,
                mode: config.mode,
                limit: config.limit,
                downloads,
                slow: SlowRule::default(),
                closed: false,
                seed_ratio: config.seed_ratio,
            }),
            events: broadcast::channel(256).0,
            client: RwLock::new(client),
            user_agent: config.user_agent,
            wake: Arc::new(Notify::new()),
            online: watch::channel(true).0,
            retry_base_ms: AtomicU64::new(1000),
            replaced: AtomicU64::new(0),
            db,
            tools: Tools::new(data_dir.join("tools")),
            torrents: Torrents::new(data_dir, config.torrent_dht),
        }
    }

    pub(crate) fn tools(&self) -> &Tools {
        &self.tools
    }

    pub(crate) fn torrents(&self) -> &Torrents {
        &self.torrents
    }

    /// Whether a finished torrent should go on sharing.
    pub(crate) fn keep_seeding(&self, id: Id) -> bool {
        let g = self.lock();
        g.downloads
            .iter()
            .find(|d| d.id == id)
            .is_some_and(|d| d.extra.seed && g.seed_ratio_left(d))
    }

    /// Cheap: `reqwest::Client` is a handle to a shared connection pool.
    pub(crate) fn client(&self) -> reqwest::Client {
        self.client
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn set_proxy(&self, proxy: &Proxy) -> crate::Result<()> {
        let client = build_client(&self.user_agent, proxy)?;
        *self.client.write().unwrap_or_else(|e| e.into_inner()) = client;
        Ok(())
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Announces a state change and checkpoints it. Call with the lock held.
    pub(crate) fn emit(&self, dl: &Download) {
        let _ = self.events.send(Event::Changed(dl.view()));
        self.db.save(vec![dl.record()]);
    }

    /// Takes a download out of the list and the database.
    pub(crate) fn forget(&self, id: Id) -> Result<Download> {
        let mut g = self.lock();
        let i = g
            .downloads
            .iter()
            .position(|d| d.id == id)
            .ok_or(Error::NotFound(id))?;
        self.db.delete(id);
        Ok(g.downloads.remove(i))
    }

    pub(crate) fn retry_base(&self) -> Duration {
        Duration::from_millis(self.retry_base_ms.load(Relaxed))
    }

    pub(crate) fn has_bytes(&self, id: Id) -> bool {
        self.lock()
            .downloads
            .iter()
            .any(|d| d.id == id && d.bytes() > 0)
    }

    /// Stops every worker and writes the final state. Interrupted downloads are saved as they
    /// were, so they come back `Queued`.
    pub(crate) fn shutdown(&self) {
        let mut g = self.lock();
        g.closed = true;
        g.downloads.iter_mut().for_each(Download::stop);
        self.db
            .save(g.downloads.iter().map(Download::record).collect());
        drop(g);
        // ponytail: blocks the thread dropping the last Engine until the writer catches up
        // (milliseconds, at shutdown only); the one place the runtime may wait on disk.
        self.db.flush();
    }

    /// Partial files are flushed to disk by the database thread before this is written.
    fn checkpoint(&self) {
        let g = self.lock();
        let active = g.downloads.iter().filter(|d| d.is_active());
        self.db.save(active.map(Download::record).collect());
    }

    /// Starts queued downloads up to each queue's limit (and sends the newest back when a
    /// queue is over it, closed or held), tops up connections, finishes complete ones.
    fn schedule(self: &Arc<Self>) {
        let mut guard = self.lock();
        let inner = &mut *guard;
        if inner.closed {
            return;
        }
        let mut allowed = inner.allowance();
        let mut slot = |queue: QueueId| {
            let i = allowed.iter().position(|(q, _)| *q == queue).unwrap_or(0);
            let free = &mut allowed[i].1;
            let ok = *free > 0;
            *free = free.saturating_sub(1);
            ok
        };
        // Running ones keep their slots first, oldest first.
        for dl in inner.downloads.iter_mut().filter(|d| d.is_active()) {
            if !slot(dl.queue) && dl.requeue() {
                debug!(id = dl.id, queue = dl.queue, "back to the queue");
                self.emit(dl);
            }
        }
        for dl in &mut inner.downloads {
            if dl.state == State::Queued && slot(dl.queue) {
                dl.begin();
                self.emit(dl);
            }
            if dl.is_active() {
                self.fill(dl, inner.mode);
            }
            // Finished torrents keep sharing (outside the queue limits) until told to stop.
            if dl.kind == Kind::Torrent
                && dl.state == State::Done
                && dl.extra.seed
                && dl.runner.is_none()
                && ratio_left(inner.seed_ratio, dl)
            {
                self.spawn_runner(dl);
            }
        }
    }

    fn fill(self: &Arc<Self>, dl: &mut Download, mode: SpeedMode) {
        if dl.kind != Kind::Direct {
            if dl.runner.is_none() && dl.state == State::Downloading {
                self.spawn_runner(dl);
            }
            return;
        }
        if dl.finishing || dl.probing {
            return;
        }
        if dl.segments.is_empty() {
            self.prepare(dl);
            return;
        }
        if dl.segments.iter().all(Segment::done) {
            if dl.running() == 0 {
                self.finish(dl);
            }
            return;
        }
        if dl.state != State::Downloading {
            return;
        }
        let allowed = usize::from(dl.connections.min(mode_connections(mode)));
        while dl.running() < allowed {
            let next = dl.segments.iter().position(|s| !s.running && !s.done());
            let Some(i) = next.or_else(|| dl.split()) else {
                break;
            };
            self.spawn_worker(dl, i);
        }
    }

    pub(crate) fn spawn_worker(self: &Arc<Self>, dl: &mut Download, i: usize) {
        dl.tokens += 1;
        let seg = &mut dl.segments[i];
        let job = Job {
            id: dl.id,
            run: dl.run,
            token: dl.tokens,
            start: seg.start,
            prog: seg.prog.clone(),
            urls: std::iter::once(&dl.fetch_url)
                .chain(&dl.extra.mirrors)
                .cloned()
                .collect(),
            offset: dl.extra.zip.as_ref().map_or(0, |z| z.offset),
            headers: dl.headers.clone(),
            resumable: dl.resumable,
            if_range: dl.validators.if_range().map(str::to_owned),
            part: part_path(&dl.path),
        };
        seg.running = true;
        seg.retrying = false;
        seg.token = job.token;
        seg.started = Instant::now();
        seg.slow_since = None;
        seg.task = Some(tokio::spawn(worker::run(self.clone(), job)).abort_handle());
    }

    /// Probes the link again and recreates the partial file, after a restart.
    fn prepare(self: &Arc<Self>, dl: &mut Download) {
        dl.probing = true;
        let (id, run, path) = (dl.id, dl.run, dl.path.clone());
        let (url, headers) = (dl.url.clone(), dl.headers.clone());
        let shared = self.clone();
        let task = tokio::spawn(async move {
            let probed: Result<(Probe, Validators)> = async {
                let (probe, validators) = probe::probe(&shared.client(), &url, &headers).await?;
                let (part, size) = (part_path(&path), probe.size);
                tokio::task::spawn_blocking(move || create_part_file(&part, size))
                    .await
                    .map_err(io::Error::other)??;
                Ok((probe, validators))
            }
            .await;
            shared.prepared(id, run, probed);
        });
        dl.tasks.push(task.abort_handle());
    }

    fn prepared(&self, id: Id, run: u64, probed: Result<(Probe, Validators)>) {
        let mut g = self.lock();
        let Some(dl) = g.current(id, run) else { return };
        dl.probing = false;
        match probed {
            Ok((probe, validators)) => {
                debug!(id, "probed again");
                // ponytail: never raised again if the server becomes resumable; keep the requested
                // count in the record if that matters.
                if !probe.resumable || probe.size.is_none() {
                    dl.connections = 1;
                }
                dl.fetch_url = probe.url;
                dl.size = match &dl.extra.zip {
                    Some(z) => Some(z.compressed),
                    None => probe.size,
                };
                dl.resumable = probe.resumable;
                dl.validators = validators;
                dl.segments = initial_segments(dl.size, dl.connections);
                self.db.save(vec![dl.record()]);
            }
            Err(e) => {
                let expired = matches!(e, Error::Status(401 | 403 | 404 | 410));
                dl.fail_with(Stop {
                    msg: e.to_string(),
                    expired,
                });
                self.emit(dl);
            }
        }
        drop(g);
        self.wake.notify_one();
    }

    /// Flushes and renames `.kdpart` to the final name off the async runtime.
    fn finish(self: &Arc<Self>, dl: &mut Download) {
        dl.finishing = true;
        let (id, run, path, host_url) = (dl.id, dl.run, dl.path.clone(), dl.url.clone());
        let referrer = dl
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("referer"))
            .map(|(_, v)| v.clone());
        let expected = dl.extra.checksum.clone();
        let unzip = dl.extra.zip.clone();
        let shared = self.clone();
        let task = tokio::spawn(async move {
            let part = part_path(&path);
            let target = path.clone();
            let done = tokio::task::spawn_blocking(move || {
                let finish = || -> io::Result<()> {
                    OpenOptions::new().write(true).open(&part)?.sync_all()?;
                    if let Some(want) = &expected
                        && checksum::file_hash(&part, want.algo)? != want.hex
                    {
                        return Err(io::Error::other(CHECKSUM_MISMATCH));
                    }
                    match &unzip {
                        Some(z) => zip::unpack(&part, &target, z)?,
                        // ponytail: overwrites a file that appeared at the final path meanwhile; re-pick the name if that bites.
                        None => std::fs::rename(&part, &target)?,
                    }
                    mark_of_the_web(&target, referrer.as_deref(), &host_url);
                    Ok(())
                };
                finish()
            })
            .await
            .unwrap_or_else(|e| Err(io::Error::other(e)));
            shared.finished(id, run, done);
        });
        dl.tasks.push(task.abort_handle());
    }

    fn finished(&self, id: Id, run: u64, done: io::Result<()>) {
        let mut g = self.lock();
        let inner = &mut *g;
        let Some(dl) = inner.current(id, run) else {
            return;
        };
        dl.finishing = false;
        let mut follow_up = None;
        match done {
            Ok(()) => {
                debug!(id, path = %dl.path.display(), "done");
                dl.state = State::Done;
                dl.error = None;
                dl.finished_at = Some(now_ms());
                dl.size = Some(dl.extra.zip.as_ref().map_or_else(|| dl.bytes(), |z| z.size));
                follow_up = dl.convert_after();
            }
            Err(e) if e.to_string() == CHECKSUM_MISMATCH => dl.fail(CHECKSUM_MISMATCH.into()),
            Err(e) => dl.fail(format!("Could not finish the file: {e}")),
        }
        self.emit(dl);
        self.queue_conversion(inner, follow_up);
        drop(g);
        self.wake.notify_one();
    }

    /// "Convert after download": the conversion is its own job in the list.
    fn queue_conversion(&self, inner: &mut Inner, follow_up: Option<(PathBuf, String, QueueId)>) {
        if let Some((input, preset, queue)) = follow_up {
            let id = inner.next_id;
            inner.next_id += 1;
            let job = convert::job(id, input, &preset, queue);
            self.emit(&job);
            inner.downloads.push(job);
        }
    }

    /// Starts the task that downloads a torrent or video, or converts a file.
    fn spawn_runner(self: &Arc<Self>, dl: &mut Download) {
        let job = runner::Job {
            id: dl.id,
            run: dl.run,
            kind: dl.kind,
            url: dl.url.clone(),
            headers: dl.headers.clone(),
            path: dl.path.clone(),
            extra: dl.extra.clone(),
            ext: dl.ext.clone(),
            finished: dl.state == State::Done,
            rate: self.limiter.rate(),
        };
        let shared = self.clone();
        dl.runner = Some(tokio::spawn(runner::run(shared, job)).abort_handle());
    }

    /// A runner reached its goal (`Ok`) or gave up. A torrent keeps running after it finishes
    /// downloading, to share; `still_running` says so.
    pub(crate) fn runner_done(
        &self,
        id: Id,
        run: u64,
        outcome: std::result::Result<runner::Finished, Stop>,
        still_running: bool,
    ) {
        let mut g = self.lock();
        let inner = &mut *g;
        let Some(dl) = inner
            .downloads
            .iter_mut()
            .find(|d| d.id == id && d.run == run)
        else {
            return;
        };
        if !still_running {
            dl.runner = None;
        }
        let follow_up = match outcome {
            Ok(f) if dl.state != State::Done => {
                debug!(id, path = %f.path.display(), "done");
                dl.state = State::Done;
                dl.error = None;
                dl.finished_at = Some(now_ms());
                if let Some(name) = f.path.file_name() {
                    dl.name = name.to_string_lossy().into_owned();
                }
                dl.path = f.path;
                let size = f
                    .size
                    .unwrap_or_else(|| dl.total().unwrap_or(0).max(dl.bytes()));
                dl.size = Some(size);
                dl.ext.set(size, size);
                dl.extra.parts.clear();
                let follow_up = dl.convert_after();
                self.emit(dl);
                follow_up
            }
            Ok(_) => {
                self.emit(dl);
                None
            }
            Err(stop) => {
                dl.fail_with(stop);
                self.emit(dl);
                None
            }
        };
        self.queue_conversion(inner, follow_up);
        drop(g);
        self.wake.notify_one();
    }

    /// A worker hit an error and is backing off. When every connection is, the download waits.
    pub(crate) fn report_retry(&self, job: &Job, msg: &str) {
        let mut g = self.lock();
        let Some(dl) = g.current(job.id, job.run) else {
            return;
        };
        let Some(s) = dl.worker_segment(job) else {
            return;
        };
        s.retrying = true;
        if dl.state == State::Downloading
            && dl.segments.iter().filter(|s| s.running).all(|s| s.retrying)
        {
            dl.state = State::WaitingForNetwork;
            dl.error = Some(msg.to_string());
            self.emit(dl);
        }
    }

    /// Bytes are flowing again after an error.
    pub(crate) fn report_flowing(&self, job: &Job) {
        let mut g = self.lock();
        let Some(dl) = g.current(job.id, job.run) else {
            return;
        };
        let Some(s) = dl.worker_segment(job) else {
            return;
        };
        s.retrying = false;
        dl.extra.needs_link = false;
        if dl.state == State::WaitingForNetwork {
            dl.state = State::Downloading;
            dl.error = None;
            self.emit(dl);
            drop(g);
            self.wake.notify_one();
        }
    }

    pub(crate) fn worker_done(&self, job: &Job, outcome: std::result::Result<(), Stop>) {
        let mut g = self.lock();
        let Some(dl) = g.current(job.id, job.run) else {
            return;
        };
        let Some(s) = dl.worker_segment(job) else {
            return;
        };
        match outcome {
            Ok(()) => {
                s.running = false;
                s.retrying = false;
                s.task = None;
            }
            Err(stop) => {
                dl.fail_with(stop);
                self.emit(dl);
            }
        }
        drop(g);
        self.wake.notify_one();
    }

    /// Updates speeds and replaces slow connections.
    fn tick(self: &Arc<Self>, secs: f64) {
        let alpha = 1.0 - (-secs / SPEED_TAU).exp();
        let now = Instant::now();
        let mut guard = self.lock();
        let inner = &mut *guard;
        if inner.closed {
            return;
        }
        let mut received = 0;
        for dl in &mut inner.downloads {
            if dl.kind != Kind::Direct {
                let (recv, sent) = (dl.ext.recv.load(Relaxed), dl.ext.sent.load(Relaxed));
                let (down, up) = (
                    recv.saturating_sub(dl.ext_last.0),
                    sent.saturating_sub(dl.ext_last.1),
                );
                received += down;
                dl.ext_last = (recv, sent);
                dl.ext_speed.0 += alpha * (down as f64 / secs - dl.ext_speed.0);
                dl.ext_speed.1 += alpha * (up as f64 / secs - dl.ext_speed.1);
                continue;
            }
            for s in &mut dl.segments {
                let recv = s.prog.recv.load(Relaxed);
                received += recv - s.last_recv;
                let speed = (recv - s.last_recv) as f64 / secs;
                s.last_recv = recv;
                s.speed += alpha * (speed - s.speed);
            }
            self.replace_slow(dl, inner.slow, now);
        }
        inner.quota.add(received, now);
    }

    /// With three or more connections, one running under a quarter of the median for a while is
    /// closed and reopened on the same range; a new connection often lands on a faster server.
    fn replace_slow(self: &Arc<Self>, dl: &mut Download, rule: SlowRule, now: Instant) {
        if dl.state != State::Downloading {
            return;
        }
        let mut speeds: Vec<f64> = dl
            .segments
            .iter()
            .filter(|s| s.running && !s.retrying)
            .map(|s| s.speed)
            .collect();
        if speeds.len() < 3 {
            return;
        }
        speeds.sort_by(f64::total_cmp);
        let floor = speeds[speeds.len() / 2] / 4.0;
        for i in 0..dl.segments.len() {
            let s = &mut dl.segments[i];
            let slow = s.running
                && !s.retrying
                && now.duration_since(s.started) >= rule.grace
                && s.speed < floor;
            if !slow {
                s.slow_since = None;
                continue;
            }
            let since = *s.slow_since.get_or_insert(now);
            let cooled = s
                .replaced_at
                .is_none_or(|t| now.duration_since(t) >= rule.cooldown);
            if now.duration_since(since) < rule.window || !cooled {
                continue;
            }
            debug!(
                id = dl.id,
                segment = s.start,
                speed = s.speed as u64,
                "replacing a slow connection"
            );
            if let Some(t) = s.task.take() {
                t.abort();
            }
            s.replaced_at = Some(now);
            self.replaced.fetch_add(1, Relaxed);
            self.spawn_worker(dl, i);
        }
    }
}

/// Runs until the engine is dropped.
pub(crate) async fn scheduler(shared: Weak<Shared>, wake: Arc<Notify>) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let (mut last, mut ticks) = (Instant::now(), 0u32);
    loop {
        let ticked = tokio::select! {
            _ = wake.notified() => false,
            _ = tick.tick() => true,
        };
        let Some(shared) = shared.upgrade() else {
            break;
        };
        if shared.lock().closed {
            break;
        }
        if ticked {
            let now = Instant::now();
            shared.tick(now.duration_since(last).as_secs_f64().max(0.001));
            last = now;
            ticks += 1;
            if ticks % CHECKPOINT_TICKS == 0 {
                shared.checkpoint();
            }
        }
        shared.schedule();
    }
}

/// One HTTP client per proxy setting.
pub(crate) fn build_client(user_agent: &str, proxy: &Proxy) -> crate::Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .user_agent(user_agent)
        // HTTP/2 would multiplex every segment over one TCP connection and lose the
        // per-connection speedup that segmented downloading exists for.
        .http1_only()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30));
    let builder = match proxy {
        // The `system-proxy` feature reads the OS settings and *_PROXY variables.
        Proxy::System => builder,
        Proxy::None => builder.no_proxy(),
        Proxy::Custom(url) => {
            let url = url.trim();
            if !["http://", "https://", "socks5://", "socks5h://"]
                .iter()
                .any(|s| url.starts_with(s))
            {
                return Err(crate::Error::InvalidProxy(format!(
                    "{url} (start it with http://, https:// or socks5://)"
                )));
            }
            let proxy =
                reqwest::Proxy::all(url).map_err(|e| crate::Error::InvalidProxy(e.to_string()))?;
            builder.proxy(proxy)
        }
    };
    builder
        .build()
        .map_err(|e| crate::Error::InvalidProxy(e.to_string()))
}

#[cfg(test)]
mod proxy_tests {
    use super::*;

    #[test]
    fn proxy_settings_build_clients_or_explain_why_not() {
        assert!(build_client("t", &Proxy::System).is_ok());
        assert!(build_client("t", &Proxy::None).is_ok());
        assert!(build_client("t", &Proxy::Custom("socks5://127.0.0.1:1080".into())).is_ok());
        assert!(
            build_client(
                "t",
                &Proxy::Custom("http://user:pass@proxy.test:8080".into())
            )
            .is_ok()
        );
        let err = build_client("t", &Proxy::Custom("proxy.test:8080".into())).unwrap_err();
        assert!(err.to_string().contains("socks5://"));
    }
}
