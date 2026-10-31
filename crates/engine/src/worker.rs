//! One task per segment: fetch its byte range, write it in place, retry on failure.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, IF_RANGE, RANGE, RETRY_AFTER};
use tracing::warn;

use crate::download::{Progress, Shared};
use crate::ftp;
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
    /// The link, then its mirrors. Each worker starts on its own one and moves on after errors.
    pub(crate) urls: Vec<String>,
    /// Where byte 0 of this download sits in the remote file (non-zero for one ZIP entry).
    pub(crate) offset: u64,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) resumable: bool,
    /// Strong ETag or Last-Modified from the probe.
    pub(crate) if_range: Option<String>,
    pub(crate) part: PathBuf,
}

/// Why a worker gave up; ends the download.
pub(crate) struct Stop {
    pub(crate) msg: String,
    /// The server refused the link itself (expired, one-time, signed in elsewhere).
    pub(crate) expired: bool,
}

enum Fail {
    /// Network trouble or a busy server: back off and retry, forever.
    Retry {
        msg: String,
        after: Option<Duration>,
    },
    /// A 4xx answer (or a web page where the file was): retried a few times, then the
    /// download fails.
    Refused(u16),
    /// Retrying cannot help.
    Fatal(String),
}

pub(crate) async fn run(shared: Arc<Shared>, job: Job) {
    let mut failures = 0u32;
    // Spread the connections over the mirrors.
    let mut source = job.token as usize % job.urls.len().max(1);
    let outcome = loop {
        wait_online(&shared).await;
        let (msg, after) = match fetch(&shared, &job, source, &mut failures).await {
            Ok(()) => break Ok(()),
            Err(Fail::Fatal(msg)) => {
                break Err(Stop {
                    msg,
                    expired: false,
                });
            }
            Err(Fail::Refused(code)) if failures + 1 >= MAX_REFUSED => {
                break Err(Stop {
                    msg: refused_message(code),
                    expired: matches!(code, 403 | 404 | 410 | EXPIRED_PAGE),
                });
            }
            Err(Fail::Refused(code)) => (format!("The server answered {code}, retrying"), None),
            Err(Fail::Retry { msg, after }) => (msg, after),
        };
        failures += 1;
        source = (source + 1) % job.urls.len().max(1);
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

/// Stands in for a status code: the server sent a web page instead of the rest of the file.
const EXPIRED_PAGE: u16 = 1;

fn refused_message(code: u16) -> String {
    match code {
        EXPIRED_PAGE => {
            "The link now leads to a web page instead of the file. Get a fresh link from the page."
                .into()
        }
        403 | 410 => {
            format!("The link expired or was refused ({code}). Get a fresh link from the page.")
        }
        404 => "The file is no longer on the server (404).".into(),
        401 => {
            "The site wants a sign-in. Add it in Settings, under Sign-ins, then try again.".into()
        }
        _ => format!("The server refused the download ({code})."),
    }
}

async fn fetch(shared: &Shared, job: &Job, source: usize, failures: &mut u32) -> Result<(), Fail> {
    let prog = &job.prog;
    if !job.resumable {
        prog.pos.store(0, Relaxed);
    }
    let pos = prog.pos.load(Relaxed);
    let end = prog.end.load(Relaxed);
    if pos >= end {
        return Ok(());
    }
    // Mirrors are other sites: no cookies, and their validators differ from the probe's.
    let primary = source == 0;
    let url = &job.urls[source.min(job.urls.len() - 1)];
    if ftp::is_ftp(url) {
        return fetch_ftp(shared, job, url, pos, primary, failures).await;
    }

    let headers: &[(String, String)] = if primary { &job.headers } else { &[] };
    let mut req = probe::with_headers(shared.client().get(url), headers);
    let signed = primary
        .then(|| {
            reqwest::Url::parse(url)
                .ok()
                .and_then(|u| shared.auth.header("GET", &u))
        })
        .flatten();
    let authed = signed.is_some();
    if let Some(h) = signed {
        req = req.header(reqwest::header::AUTHORIZATION, h);
    }
    let mut guarded = false;
    // `offset` places a part of a file (one entry of a ZIP) inside the remote file.
    let at = job.offset + pos;
    if job.resumable {
        let last = if end == u64::MAX {
            String::new()
        } else {
            (job.offset + end - 1).to_string()
        };
        req = req.header(RANGE, format!("bytes={at}-{last}"));
        // Once anything is on disk, the rest must come from the same version of the file.
        if let Some(v) = job.if_range.as_ref().filter(|_| primary)
            && (pos > 0 || shared.has_bytes(job.id))
        {
            req = req.header(IF_RANGE, v.as_str());
            guarded = true;
        }
    }
    let resp = req.send().await.map_err(network)?;
    // Expired links often redirect to a sign-in or "link expired" page, which answers a
    // range request with a whole web page.
    let page = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if page && pos > 0 && resp.status() == reqwest::StatusCode::OK {
        return Err(Fail::Refused(EXPIRED_PAGE));
    }
    match resp.status().as_u16() {
        206 => {
            if let Some((first, ..)) = probe::content_range(resp.headers())
                && first != at
            {
                return Err(Fail::Fatal(
                    "The server sent the wrong part of the file.".into(),
                ));
            }
        }
        200 if guarded => return Err(Fail::Fatal(FILE_CHANGED.into())),
        200 if at == 0 => {}
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
        // The site asks for a sign-in, or a Digest nonce went stale: retry right away. A
        // refused password is a refusal like any other.
        401 if primary
            && probe::www_authenticate(resp.headers())
                .filter(|c| !authed || c.to_ascii_lowercase().contains("stale=true"))
                .zip(reqwest::Url::parse(url).ok())
                .is_some_and(|(c, u)| shared.auth.learn(&u, &c)) =>
        {
            return Err(Fail::Retry {
                msg: "Signing in to the site".into(),
                after: Some(Duration::ZERO),
            });
        }
        code @ 400..=499 => return Err(Fail::Refused(code)),
        code => return Err(Fail::Fatal(format!("Unexpected server answer ({code})."))),
    }
    pump(shared, job, pos, failures, Body::Http(resp)).await
}

/// Where a segment's bytes come from.
enum Body {
    Http(reqwest::Response),
    Ftp {
        // Kept open until the transfer ends; closing it early aborts the transfer.
        _session: Box<ftp::Ftp>,
        data: Box<suppaftp::tokio::TransferStream<suppaftp::tokio::AsyncRustlsStream>>,
        buf: Vec<u8>,
    },
}

impl Body {
    async fn next(&mut self) -> Result<Option<bytes::Bytes>, Fail> {
        match self {
            Self::Http(resp) => resp.chunk().await.map_err(network),
            Self::Ftp { data, buf, .. } => {
                use tokio::io::AsyncReadExt;
                match data.read(buf).await {
                    Ok(0) => Ok(None),
                    Ok(n) => Ok(Some(bytes::Bytes::copy_from_slice(&buf[..n]))),
                    Err(e) => Err(Fail::Retry {
                        msg: format!("Connection problem ({e}), retrying"),
                        after: None,
                    }),
                }
            }
        }
    }
}

/// One FTP session per segment: `REST` to the segment's position, then `RETR` until its end,
/// then hang up (the server sees an aborted transfer, which is fine).
async fn fetch_ftp(
    shared: &Shared,
    job: &Job,
    url: &str,
    pos: u64,
    primary: bool,
    failures: &mut u32,
) -> Result<(), Fail> {
    let target = reqwest::Url::parse(url)
        .map_err(|e| Fail::Fatal(e.to_string()))
        .and_then(|u| ftp::target(&u, &shared.auth).map_err(|e| Fail::Fatal(e.to_string())))?;
    let mut session = ftp::connect(&target).await.map_err(ftp_fail)?;
    let at = job.offset + pos;
    if at > 0 {
        if let Some(v) = job.if_range.as_ref().filter(|_| primary)
            && ftp::modified(&mut session, &target.path).await.as_ref() != Some(v)
        {
            return Err(Fail::Fatal(FILE_CHANGED.into()));
        }
        session.resume_transfer(at as usize).await.map_err(|e| {
            if ftp::not_resumable(&e) {
                Fail::Fatal("The server stopped accepting resume requests (REST).".into())
            } else {
                ftp_fail(e)
            }
        })?;
    }
    let data = session
        .retr_as_stream(&target.path)
        .await
        .map_err(ftp_fail)?;
    let body = Body::Ftp {
        _session: Box::new(session),
        data: Box::new(data),
        buf: vec![0u8; 64 * 1024],
    };
    pump(shared, job, pos, failures, body).await
}

fn ftp_fail(e: suppaftp::FtpError) -> Fail {
    match ftp::code(&e) {
        Some(c) if ftp::transient(&e) => Fail::Retry {
            msg: format!("The FTP server is busy ({c}), retrying"),
            after: None,
        },
        Some(c @ (530 | 532 | 550)) => Fail::Refused(ftp::as_http(c)),
        Some(c) => Fail::Fatal(format!("The FTP server refused the download ({c}).")),
        None => Fail::Retry {
            msg: format!("Connection problem ({e}), retrying"),
            after: None,
        },
    }
}

/// Streams a response into the segment, writing at its position until its (possibly
/// lowered) end, and keeping what arrived when the connection breaks.
async fn pump(
    shared: &Shared,
    job: &Job,
    mut pos: u64,
    failures: &mut u32,
    mut body: Body,
) -> Result<(), Fail> {
    let prog = &job.prog;
    let file = open(&job.part).await.map_err(file_error)?;
    let mut buf = Vec::with_capacity(BUF + 64 * 1024);
    loop {
        let chunk = match body.next().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(e) => {
                // Keep what already arrived; the retry continues after it.
                flush(&file, prog, &mut pos, &mut buf).await?;
                return Err(e);
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
