//! Playing a download while it downloads: a small HTTP server on 127.0.0.1 that feeds the
//! in-app player. Each request carries a random token, so other programs on the computer
//! can't read downloads through it. Reads past what is downloaded wait for the bytes and
//! ask the engine to fetch from there first (seeking in the player works).

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use klyxdown_engine::{Engine, Id, Kind};

/// Gives up on bytes that don't arrive (stalled server, paused download).
const PATIENCE: Duration = Duration::from_secs(90);
const CHUNK: u64 = 256 * 1024;

#[derive(Clone)]
struct Feed {
    engine: Engine,
    token: String,
}

pub struct Preview {
    port: u16,
    token: String,
}

impl Preview {
    pub fn url(&self, id: Id) -> String {
        format!("http://127.0.0.1:{}/{}/{id}", self.port, self.token)
    }
}

pub fn token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source works");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn start(engine: Engine) -> io::Result<Preview> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let feed = Feed {
        engine,
        token: token(),
    };
    let preview = Preview {
        port,
        token: feed.token.clone(),
    };
    let app = Router::new()
        .route("/{token}/{id}", get(play))
        .with_state(feed);
    tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::warn!("preview server stopped: {e}");
        }
    });
    Ok(preview)
}

/// `bytes=a-` or `bytes=a-b`, clamped to the size.
fn range(h: &HeaderMap, size: u64) -> Option<(u64, u64)> {
    let spec = h
        .get(header::RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let last = size.checked_sub(1)?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => (size.saturating_sub(n.parse().ok()?), last),
        (a, "") => (a.parse().ok()?, last),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(last)),
    };
    (start <= end).then_some((start, end))
}

fn mime(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

fn partial(start: u64, end: u64, size: u64, name: &str, body: Body) -> Response {
    Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::CONTENT_TYPE, mime(name))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"))
        .header(header::CONTENT_LENGTH, end - start + 1)
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn play(
    State(feed): State<Feed>,
    UrlPath((token, id)): UrlPath<(String, Id)>,
    headers: HeaderMap,
) -> Response {
    // Same length every time; a wrong token is just "not found".
    if token.len() != feed.token.len() || token != feed.token {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(view) = feed.engine.list().into_iter().find(|d| d.id == id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if view.kind == Kind::Torrent {
        return play_torrent(&feed.engine, id, &headers).await;
    }
    let Ok((_, Some(size), _)) = feed.engine.preview_at(id, 0) else {
        return (StatusCode::CONFLICT, "The size is not known yet.").into_response();
    };
    let (start, end) = range(&headers, size).unwrap_or((0, size.saturating_sub(1)));
    let engine = feed.engine.clone();
    let stream = futures_util::stream::unfold((engine, start), move |(engine, pos)| async move {
        if pos > end {
            return None;
        }
        let chunk = next_chunk(&engine, id, pos, end).await;
        let next = pos + chunk.as_ref().map_or(0, |c| c.len() as u64);
        // An error ends the stream after it is reported.
        let next = if chunk.is_err() { end + 1 } else { next };
        Some((chunk, (engine, next)))
    });
    partial(start, end, size, &view.name, Body::from_stream(stream))
}

/// Waits until bytes at `pos` exist (asking for them first), then reads up to a chunk.
async fn next_chunk(engine: &Engine, id: Id, pos: u64, end: u64) -> io::Result<Vec<u8>> {
    let waited = Instant::now();
    let mut asked = false;
    loop {
        let (path, _, ready) = engine.preview_at(id, pos).map_err(io::Error::other)?;
        if ready > 0 {
            let len = ready.min(CHUNK).min(end - pos + 1) as usize;
            return tokio::task::spawn_blocking(move || read_at(&path, pos, len))
                .await
                .map_err(io::Error::other)?;
        }
        if !asked {
            let _ = engine.prioritize(id, pos);
            asked = true;
        }
        if waited.elapsed() > PATIENCE {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the download stopped feeding the player",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn read_at(path: &Path, pos: u64, len: usize) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; len];
    #[cfg(unix)]
    std::os::unix::fs::FileExt::read_exact_at(&file, &mut buf, pos)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < len {
            let n = file.seek_read(&mut buf[done..], pos + done as u64)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
    }
    Ok(buf)
}

async fn play_torrent(engine: &Engine, id: Id, headers: &HeaderMap) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let (name, size, mut stream) = match engine.torrent_stream(id).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::CONFLICT, e.to_string()).into_response(),
    };
    let (start, end) = range(headers, size).unwrap_or((0, size.saturating_sub(1)));
    if let Err(e) = stream.seek(io::SeekFrom::Start(start)).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let body = tokio_util::io::ReaderStream::new(stream.take(end - start + 1));
    partial(start, end, size, &name, Body::from_stream(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        let h = |v: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::RANGE, v.parse().unwrap());
            m
        };
        assert_eq!(range(&h("bytes=0-"), 100), Some((0, 99)));
        assert_eq!(range(&h("bytes=10-19"), 100), Some((10, 19)));
        assert_eq!(range(&h("bytes=90-500"), 100), Some((90, 99)));
        assert_eq!(range(&h("bytes=-10"), 100), Some((90, 99)));
        assert_eq!(range(&h("bytes=50-10"), 100), None);
        assert_eq!(range(&HeaderMap::new(), 100), None);
        assert_eq!(mime("Film.MKV"), "video/x-matroska");
        assert_eq!(token().len(), 32);
    }
}
