//! One task per segment: fetch its byte range, write it in place, retry on failure.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

use reqwest::header::{IF_RANGE, RANGE, RETRY_AFTER};
use tracing::warn;

use crate::download::{Progress, Shared};
use crate::probe::{self, describe};

const BUF: usize = 256 * 1024;
/// Consecutive 4xx answers (other than 408/429) before the download fails.
const MAX_REFUSED: u32 = 8;
/// Seconds; a server asking for longer is waited on for this long.
const MAX_RETRY_AFTER: u64 = 600;
const FILE_CHANGED: &str = "The file changed on the server since this download started. \
                            Restart it to get the current version.";

pub(crate) struct Job {
    pub(crate) id: u64,
    pub(crate) run: u64,
    pub(crate) token: u64,
    pub(crate) start: u64,
    pub(crate) prog: Arc<Progress>,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) resumable: bool,
    /// Strong ETag or Last-Modified from the probe.
    pub(crate) if_range: Option<String>,
    pub(crate) part: PathBuf,
}

enum Fail {
    /// Network trouble or a busy server: back off and retry, forever.
    Retry {
        msg: String,
        after: Option<Duration>,
    },
    /// A 4xx answer: retried a few times, then the download fails.
    Refused(u16),
    /// Retrying cannot help.
    Fatal(String),
}

pub(crate) async fn run(shared: Arc<Shared>, job: Job) {
    let mut failures = 0u32;
    let outcome = loop {
        wait_online(&shared).await;
        let (msg, after) = match fetch(&shared, &job, &mut failures).await {
            Ok(()) => break Ok(()),
            Err(Fail::Fatal(msg)) => break Err(msg),
            Err(Fail::Refused(code)) if failures + 1 >= MAX_REFUSED => {
                break Err(refused_message(code));
            }
            Err(Fail::Refused(code)) => (format!("The server answered {code}, retrying"), None),
            Err(Fail::Retry { msg, after }) => (msg, after),
        };
        failures += 1;
        warn!(id = job.id, segment = job.start, failures, "{msg}");
        // Subscribed before reporting, so a `set_online` reacting to the report isn't missed.
        // Going online (or offline) cuts the wait short; `wait_online` then decides.
        let mut online = shared.online.subscribe();
        shared.report_retry(&job, &msg);
        let base = shared.retry_base();
        let delay = after.unwrap_or_else(|| (base * (1 << (failures - 1).min(5))).min(base * 30));
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = online.changed() => {}
        }
    };
    shared.worker_done(&job, outcome);
}

async fn wait_online(shared: &Shared) {
    let mut online = shared.online.subscribe();
    while !*online.borrow_and_update() {
        if online.changed().await.is_err() {
            return;
        }
    }
}

fn refused_message(code: u16) -> String {
    match code {
        403 | 410 => {
            format!("The link expired or was refused ({code}). Get a fresh link from the page.")
        }
        404 => "The file is no longer on the server (404).".into(),
        _ => format!("The server refused the download ({code})."),
    }
}

async fn fetch(shared: &Shared, job: &Job, failures: &mut u32) -> Result<(), Fail> {
    let prog = &job.prog;
    if !job.resumable {
        prog.pos.store(0, Relaxed);
    }
    let mut pos = prog.pos.load(Relaxed);
    let end = prog.end.load(Relaxed);
    if pos >= end {
        return Ok(());
    }

    let mut req = probe::with_headers(shared.client().get(&job.url), &job.headers);
    let mut guarded = false;
    if job.resumable {
        let last = if end == u64::MAX {
            String::new()
        } else {
            (end - 1).to_string()
        };
        req = req.header(RANGE, format!("bytes={pos}-{last}"));
        // Once anything is on disk, the rest must come from the same version of the file.
        if let Some(v) = &job.if_range
            && (pos > 0 || shared.has_bytes(job.id))
        {
            req = req.header(IF_RANGE, v.as_str());
            guarded = true;
        }
    }
    let mut resp = req.send().await.map_err(network)?;
    match resp.status().as_u16() {
        206 => {
            if let Some((first, ..)) = probe::content_range(resp.headers())
                && first != pos
            {
                return Err(Fail::Fatal(
                    "The server sent the wrong part of the file.".into(),
                ));
            }
        }
        200 if guarded => return Err(Fail::Fatal(FILE_CHANGED.into())),
        200 if pos == 0 => {}
        200 => {
            return Err(Fail::Fatal(
                "The server stopped accepting resume requests (200 to a range).".into(),
            ));
        }
        code @ (408 | 429 | 500..=599) => {
            let after = probe::header_u64(resp.headers(), RETRY_AFTER)
                .map(|secs| Duration::from_secs(secs.min(MAX_RETRY_AFTER)));
            return Err(Fail::Retry {
                msg: format!("The server is busy ({code}), retrying"),
                after,
            });
        }
        code @ 400..=499 => return Err(Fail::Refused(code)),
        code => return Err(Fail::Fatal(format!("Unexpected server answer ({code})."))),
    }

    let file = open(&job.part).await.map_err(file_error)?;
    let mut buf = Vec::with_capacity(BUF + 64 * 1024);
    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(e) => {
                // Keep what already arrived; the retry continues after it.
                flush(&file, prog, &mut pos, &mut buf).await?;
                return Err(network(e));
            }
        };
        if *failures > 0 {
            *failures = 0;
            shared.report_flowing(job);
        }
        prog.recv.fetch_add(chunk.len() as u64, Relaxed);
        shared.limiter.take(chunk.len()).await;
        buf.extend_from_slice(&chunk);
        if buf.len() >= BUF || pos + buf.len() as u64 >= prog.end.load(Relaxed) {
            flush(&file, prog, &mut pos, &mut buf).await?;
            if pos >= prog.end.load(Relaxed) {
                return Ok(());
            }
        }
    }
    flush(&file, prog, &mut pos, &mut buf).await?;
    match prog.end.load(Relaxed) {
        u64::MAX => {
            prog.end.store(pos, Relaxed);
            Ok(())
        }
        end if pos >= end => Ok(()),
        _ => Err(Fail::Retry {
            msg: "The connection closed early, retrying".into(),
            after: None,
        }),
    }
}

/// Writes the buffer at `pos`, cut at the segment's current `end` (a split may have lowered it).
async fn flush(
    file: &Arc<File>,
    prog: &Progress,
    pos: &mut u64,
    buf: &mut Vec<u8>,
) -> Result<(), Fail> {
    let n = (buf.len() as u64).min(prog.end.load(Relaxed).saturating_sub(*pos)) as usize;
    if n > 0 {
        let (file, data, at) = (file.clone(), std::mem::take(buf), *pos);
        *buf =
            tokio::task::spawn_blocking(move || write_all_at(&file, &data[..n], at).map(|()| data))
                .await
                .map_err(|e| file_error(io::Error::other(e)))?
                .map_err(file_error)?;
        *pos += n as u64;
        prog.pos.store(*pos, Relaxed);
    }
    buf.clear();
    Ok(())
}

async fn open(part: &Path) -> io::Result<Arc<File>> {
    let part = part.to_owned();
    let file = tokio::task::spawn_blocking(move || OpenOptions::new().write(true).open(part))
        .await
        .map_err(io::Error::other)??;
    Ok(Arc::new(file))
}

#[cfg(windows)]
fn write_all_at(file: &File, mut data: &[u8], mut at: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !data.is_empty() {
        let n = file.seek_write(data, at)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        data = &data[n..];
        at += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn write_all_at(file: &File, data: &[u8], at: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, data, at)
}

fn network(e: reqwest::Error) -> Fail {
    if e.is_builder() || e.is_redirect() {
        return Fail::Fatal(format!("Bad request: {}", describe(&e)));
    }
    Fail::Retry {
        msg: format!("Connection problem ({}), retrying", describe(&e)),
        after: None,
    }
}

fn file_error(e: io::Error) -> Fail {
    Fail::Fatal(format!("File error: {e}"))
}
