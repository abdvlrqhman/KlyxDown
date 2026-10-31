//! Downloads one task drives from start to end: torrents (librqbit), video and audio from
//! web pages (yt-dlp) and conversions (ffmpeg). The task reports progress through [`Ext`];
//! aborting it (pause, remove) kills the child process or pauses the torrent.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use crate::download::{Extra, Shared};
use crate::worker::Stop;
use crate::{Id, Kind, convert, media, torrent};

/// Progress a runner reports, read by the scheduler without the engine lock.
#[derive(Default)]
pub(crate) struct Ext {
    /// Work done and the whole job: bytes, or milliseconds of media for conversions.
    pub(crate) done: AtomicU64,
    /// 0 while unknown.
    pub(crate) total: AtomicU64,
    /// Bytes received from the network so far (speed and quota).
    pub(crate) recv: AtomicU64,
    /// Bytes shared with other people (torrents).
    pub(crate) sent: AtomicU64,
    pub(crate) peers: AtomicU64,
    /// What it is doing beyond downloading ("Merging audio and video").
    detail: Mutex<Option<String>>,
    /// Temporary files it writes, removed with the download.
    pub(crate) parts: Mutex<Vec<PathBuf>>,
}

impl Ext {
    pub(crate) fn detail(&self) -> Option<String> {
        self.detail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn set_detail(&self, text: Option<&str>) {
        *self.detail.lock().unwrap_or_else(|e| e.into_inner()) = text.map(str::to_owned);
    }

    pub(crate) fn add_part(&self, path: PathBuf) {
        let mut parts = self.parts.lock().unwrap_or_else(|e| e.into_inner());
        if !parts.contains(&path) {
            parts.push(path);
        }
    }

    pub(crate) fn set(&self, done: u64, total: u64) {
        self.done.store(done, Relaxed);
        self.total.store(total, Relaxed);
    }
}

/// Everything a runner needs, copied out of the download so it runs without the lock.
pub(crate) struct Job {
    pub(crate) id: Id,
    pub(crate) run: u64,
    pub(crate) kind: Kind,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) path: PathBuf,
    pub(crate) extra: Extra,
    pub(crate) ext: Arc<Ext>,
    /// Already complete: a torrent that only shares now.
    pub(crate) finished: bool,
    /// Bytes per second, from the speed limit and mode.
    pub(crate) rate: Option<u64>,
}

/// Where the finished file (or torrent folder) ended up, and its size.
pub(crate) struct Finished {
    pub(crate) path: PathBuf,
    pub(crate) size: Option<u64>,
}

pub(crate) async fn run(shared: Arc<Shared>, job: Job) {
    let (id, run) = (job.id, job.run);
    let mut outcome = match job.kind {
        Kind::Media => media::run(&shared, &job).await,
        Kind::Convert => convert::run(&shared, &job).await,
        // Reports completion itself and keeps sharing afterwards.
        Kind::Torrent => torrent::run(&shared, &job).await,
        Kind::Direct => Err(Stop {
            msg: "Not a runner download.".into(),
            expired: false,
        }),
    };
    // Videos and conversions are files like any other: marked as downloaded, then scanned.
    if let Ok(f) = &outcome
        && job.kind != Kind::Torrent
        && f.path.is_file()
    {
        let (path, url, scan) = (f.path.clone(), job.url.clone(), shared.scan_setting());
        let checked = tokio::task::spawn_blocking(move || {
            crate::download::mark_of_the_web(&path, None, &url);
            crate::download::check_with_antivirus(scan.as_ref(), &path, &url, None)
        })
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)));
        if let Err(e) = checked {
            outcome = Err(Stop {
                msg: e.to_string(),
                expired: false,
            });
        }
    }
    shared.runner_done(id, run, outcome, false);
}

/// The last error line a tool printed, for the user.
pub(crate) fn last_error(stderr: &str, tool: &str) -> String {
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("ERROR:") || l.contains("Error") || l.contains("error"))
        .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
        .unwrap_or_else(|| format!("{tool} stopped without saying why."))
}
