//! End-to-end tests against a local axum server.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State as Srv;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{StreamExt, stream};
use klyxdown_engine::{
    AddRequest, Config, DownloadView, Engine, Error, Event, Hold, MAIN_QUEUE, Queue, Quota,
    Schedule, SpeedMode, State,
};

const SIZE: usize = 6 * 1024 * 1024 + 12_345;
const CHUNK: usize = 16 * 1024;

/// Deterministic and without a short period, so a misplaced segment can't go unnoticed.
static BODY: LazyLock<Vec<u8>> = LazyLock::new(|| {
    (0..SIZE as u64)
        .map(|i| {
            let mut x = i ^ 0x9E37_79B9_7F4A_7C15;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (x ^ (x >> 31)) as u8
        })
        .collect()
});

/// A small archive: "readme.txt" stored, "data/body.bin" (the start of BODY) deflated.
static ARCHIVE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    klyxdown_engine::zip::build_for_tests(&[
        ("readme.txt", b"hello from the archive", false),
        ("data/body.bin", &BODY[..300_000], true),
    ])
});

/// Any bytes, with byte ranges.
fn ranged(h: &HeaderMap, data: &'static [u8]) -> Response {
    let len = data.len();
    let range = h
        .get(header::RANGE)
        .and_then(|v| {
            v.to_str()
                .ok()?
                .strip_prefix("bytes=")?
                .split_once('-')
                .map(|(a, b)| (a.to_string(), b.to_string()))
        })
        .and_then(|(a, b)| {
            Some((
                a.parse::<usize>().ok()?,
                b.parse::<usize>().unwrap_or(len - 1).min(len - 1),
            ))
        });
    match range {
        Some((a, b)) if a <= b => Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {a}-{b}/{len}"))
            .header(header::CONTENT_LENGTH, b - a + 1)
            .body(Body::from(Bytes::from_static(&data[a..=b])))
            .unwrap(),
        _ => Response::builder()
            .header(header::CONTENT_LENGTH, len)
            .body(Body::from(Bytes::from_static(data)))
            .unwrap(),
    }
}

/// Where the hard-throttled range of `/stall.bin` starts: the third of four segments.
const STALL_START: usize = SIZE * 2 / 4;

#[derive(Default)]
struct Server {
    /// Range ends that `/flaky.bin` already broke once.
    flaky_seen: Mutex<HashSet<usize>>,
    /// Requests to `/changing.bin`; its ETag changes after the first one (the probe).
    changing_hits: AtomicUsize,
    stalled: AtomicBool,
    mirror_hits: AtomicUsize,
}

type St = Arc<Server>;

fn range(h: &HeaderMap) -> Option<(usize, usize)> {
    let v = h
        .get(header::RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes=")?;
    let (a, b) = v.split_once('-')?;
    let a: usize = a.parse().ok()?;
    let b = if b.is_empty() {
        SIZE - 1
    } else {
        b.parse::<usize>().ok()?.min(SIZE - 1)
    };
    (a <= b).then_some((a, b))
}

fn respond(h: &HeaderMap, honor_range: bool, body: impl FnOnce(usize, usize) -> Body) -> Response {
    let ranged = if honor_range { range(h) } else { None };
    let (a, b) = ranged.unwrap_or((0, SIZE - 1));
    let mut r = Response::builder().header(header::CONTENT_LENGTH, b - a + 1);
    r = match ranged {
        Some(_) => r
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {a}-{b}/{SIZE}")),
        None => r.status(StatusCode::OK),
    };
    r.body(body(a, b)).unwrap()
}

fn whole(a: usize, b: usize) -> Body {
    Body::from(Bytes::from_static(&BODY[a..=b]))
}

/// Streams `a..=b` in small chunks, sleeping `delay` before each; errors out after `fail_after` bytes.
fn trickle(a: usize, b: usize, delay: Duration, fail_after: Option<usize>) -> Body {
    let chunks = (a..=b)
        .step_by(CHUNK)
        .map(move |s| (s, (s + CHUNK - 1).min(b)));
    Body::from_stream(stream::iter(chunks).then(move |(s, e)| async move {
        tokio::time::sleep(delay).await;
        if fail_after.is_some_and(|f| s - a >= f) {
            return Err(std::io::Error::other("dropped on purpose"));
        }
        Ok(Bytes::from_static(&BODY[s..=e]))
    }))
}

async fn server() -> String {
    server_with_state().await.0
}

async fn server_with_state() -> (String, St) {
    let state = St::default();
    let app = Router::new()
        .route(
            "/file.bin",
            get(|h: HeaderMap| async move {
                let mut r = respond(&h, true, whole);
                let disposition = r#"attachment; filename="sample file.bin""#.parse().unwrap();
                r.headers_mut()
                    .insert(header::CONTENT_DISPOSITION, disposition);
                r
            }),
        )
        .route(
            "/archive.zip",
            get(|h: HeaderMap| async move { ranged(&h, &ARCHIVE) }),
        )
        // The same file as /file.bin, on "another server".
        .route(
            "/mirror.bin",
            get(|Srv(st): Srv<St>, h: HeaderMap| async move {
                st.mirror_hits.fetch_add(1, SeqCst);
                respond(&h, true, whole)
            }),
        )
        // A different, small file.
        .route(
            "/small.bin",
            get(|| async {
                Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_RANGE, "bytes 0-0/1000")
                    .header(header::CONTENT_LENGTH, 1)
                    .body(Body::from(vec![0u8]))
                    .unwrap()
            }),
        )
        // About 320 KB/s per connection: stays running for many seconds.
        .route(
            "/crawl.bin",
            get(|h: HeaderMap| async move {
                respond(&h, true, |a, b| {
                    trickle(a, b, Duration::from_millis(50), None)
                })
            }),
        )
        .route(
            "/norange.bin",
            get(|h: HeaderMap| async move { respond(&h, false, whole) }),
        )
        // Throttled, except the range that starts at 0, so the first segment finishes early.
        .route(
            "/slow.bin",
            get(|h: HeaderMap| async move {
                respond(&h, true, |a, b| {
                    if a == 0 {
                        whole(a, b)
                    } else {
                        trickle(a, b, Duration::from_millis(5), None)
                    }
                })
            }),
        )
        .route(
            "/flaky.bin",
            get(|Srv(st): Srv<St>, h: HeaderMap| async move {
                respond(&h, true, |a, b| {
                    if st.flaky_seen.lock().unwrap().insert(b) {
                        trickle(a, b, Duration::ZERO, Some(64 * 1024))
                    } else {
                        whole(a, b)
                    }
                })
            }),
        )
        // The probe passes, every real request is refused: an expired link.
        .route(
            "/gone.bin",
            get(|h: HeaderMap| async move {
                match range(&h) {
                    Some((0, 0)) => respond(&h, true, whole),
                    _ => StatusCode::FORBIDDEN.into_response(),
                }
            }),
        )
        // Honors If-Range; the file "changes" right after the probe.
        .route(
            "/changing.bin",
            get(|Srv(st): Srv<St>, h: HeaderMap| async move {
                let etag = if st.changing_hits.fetch_add(1, SeqCst) == 0 {
                    "\"v1\""
                } else {
                    "\"v2\""
                };
                let stale = h.get(header::IF_RANGE).is_some_and(|v| v != etag);
                let mut r = respond(&h, !stale, whole);
                r.headers_mut().insert(header::ETAG, etag.parse().unwrap());
                r
            }),
        )
        // Everything trickles; the first request for the third segment barely moves.
        .route(
            "/stall.bin",
            get(|Srv(st): Srv<St>, h: HeaderMap| async move {
                respond(&h, true, |a, b| {
                    let stall = a == STALL_START && !st.stalled.swap(true, SeqCst);
                    let delay = if stall { 400 } else { 10 };
                    trickle(a, b, Duration::from_millis(delay), None)
                })
            }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), state)
}

fn engine() -> Engine {
    let engine = Engine::new(Config {
        mode: SpeedMode::Full,
        ..Config::default()
    });
    engine.set_retry_base_for_tests(10);
    engine
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("klyxdown-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn request(url: String, dir: &Path, connections: u8) -> AddRequest {
    AddRequest {
        url,
        dir: dir.to_path_buf(),
        file_name: None,
        connections,
        start: true,
        ..AddRequest::default()
    }
}

fn view(engine: &Engine, id: u64) -> DownloadView {
    engine
        .list()
        .into_iter()
        .find(|d| d.id == id)
        .expect("download exists")
}

async fn wait_for(
    engine: &Engine,
    id: u64,
    what: &str,
    done: impl Fn(&DownloadView) -> bool,
) -> DownloadView {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let v = view(engine, id);
        if done(&v) {
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {v:#?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_state(engine: &Engine, id: u64, state: State) -> DownloadView {
    wait_for(engine, id, &format!("{state:?}"), |v| v.state == state).await
}

fn assert_complete(v: &DownloadView) {
    assert_eq!(v.state, State::Done);
    assert_eq!(v.bytes, SIZE as u64);
    assert!(v.finished_at.is_some());
    assert!(
        std::fs::read(&v.path).unwrap() == *BODY,
        "file content differs"
    );
    let mut part = v.path.clone().into_os_string();
    part.push(".kdpart");
    assert!(!Path::new(&part).exists(), ".kdpart left behind");
    // The file map covers the whole file without gaps or overlaps.
    let mut at = 0;
    for s in &v.segments {
        assert_eq!(s.start, at, "segments: {:?}", v.segments);
        assert_eq!(s.pos, s.end);
        at = s.end;
    }
    assert_eq!(at, SIZE as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_reads_name_size_and_range_support() {
    let base = server().await;
    let engine = engine();

    let p = engine.probe(&format!("{base}/file.bin")).await.unwrap();
    assert_eq!(
        (p.name.as_str(), p.size, p.resumable),
        ("sample file.bin", Some(SIZE as u64), true)
    );
    assert_eq!(p.host, "127.0.0.1");

    let p = engine.probe(&format!("{base}/norange.bin")).await.unwrap();
    assert_eq!(
        (p.name.as_str(), p.size, p.resumable),
        ("norange.bin", Some(SIZE as u64), false)
    );

    assert!(matches!(
        engine.probe(&format!("{base}/missing")).await,
        Err(Error::Status(404))
    ));
    assert!(matches!(
        engine.probe("file:///etc/passwd").await,
        Err(Error::InvalidUrl(_))
    ));
    assert!(matches!(
        engine.probe("not a link").await,
        Err(Error::InvalidUrl(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_connection_download_is_byte_identical() {
    let base = server().await;
    let engine = engine();
    let mut events = engine.subscribe();
    let dir = temp_dir("multi");

    let id = engine
        .add(request(format!("{base}/file.bin"), &dir, 4))
        .await
        .unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    assert_complete(&v);
    assert_eq!(v.name, "sample file.bin");
    assert_eq!((v.connections, v.segments.len()), (4, 4));

    let mut states = Vec::new();
    while let Ok(Event::Changed(v)) = events.try_recv() {
        states.push(v.state);
    }
    assert_eq!(states.first(), Some(&State::Queued));
    assert_eq!(states.last(), Some(&State::Done));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_range_server_uses_one_connection() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("norange");

    let id = engine
        .add(request(format!("{base}/norange.bin"), &dir, 8))
        .await
        .unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    assert_complete(&v);
    assert!(!v.resumable);
    assert_eq!((v.connections, v.segments.len()), (1, 1));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_then_resume_completes() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("pause");

    let id = engine
        .add(request(format!("{base}/slow.bin"), &dir, 4))
        .await
        .unwrap();
    // Wait until a throttled segment has written something.
    wait_for(&engine, id, "progress on a slow segment", |v| {
        v.segments[1..].iter().any(|s| s.pos > s.start)
    })
    .await;
    engine.pause(id).unwrap();
    let paused = view(&engine, id);
    assert_eq!(paused.state, State::Paused);
    assert!(paused.bytes < SIZE as u64);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        view(&engine, id).bytes,
        paused.bytes,
        "bytes moved while paused"
    );

    engine.resume(id).unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    assert_complete(&v);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_server_completes_through_retries() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("flaky");

    let id = engine
        .add(request(format!("{base}/flaky.bin"), &dir, 4))
        .await
        .unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    assert_complete(&v);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn going_online_retries_immediately() {
    let base = server().await;
    let engine = engine();
    // Without set_online(true) the retry would wait a minute.
    engine.set_retry_base_for_tests(60_000);
    let dir = temp_dir("online");

    let id = engine
        .add(request(format!("{base}/flaky.bin"), &dir, 1))
        .await
        .unwrap();
    let waiting = wait_state(&engine, id, State::WaitingForNetwork).await;
    assert!(waiting.error.is_some());
    engine.set_online(true);
    let v = wait_state(&engine, id, State::Done).await;
    assert_complete(&v);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_link_fails_with_message() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("gone");

    let id = engine
        .add(request(format!("{base}/gone.bin"), &dir, 2))
        .await
        .unwrap();
    let v = wait_state(&engine, id, State::Failed).await;
    let error = v.error.unwrap();
    assert!(
        error.contains("expired") && error.contains("403"),
        "{error}"
    );

    engine.remove(id, true).unwrap();
    assert!(engine.list().is_empty());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "partial file not deleted"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn name_collision_gets_a_number() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("collision");
    let paused = |url| AddRequest {
        start: false,
        ..request(url, &dir, 4)
    };

    let a = engine
        .add(paused(format!("{base}/file.bin")))
        .await
        .unwrap();
    let b = engine
        .add(paused(format!("{base}/file.bin")))
        .await
        .unwrap();
    let (a, b) = (view(&engine, a), view(&engine, b));
    assert_eq!(a.state, State::Paused);
    assert_eq!(a.path, dir.join("sample file.bin"));
    assert_eq!(b.path, dir.join("sample file (1).bin"));
    assert_eq!(b.name, "sample file (1).bin");
    assert_eq!(
        std::fs::metadata(dir.join("sample file (1).bin.kdpart"))
            .unwrap()
            .len(),
        SIZE as u64
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn free_connection_splits_the_largest_segment() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("split");

    // Two 3 MiB halves: the first finishes at once, the second is throttled with > 2 MiB left.
    let id = engine
        .add(request(format!("{base}/slow.bin"), &dir, 2))
        .await
        .unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    assert_complete(&v);
    assert!(v.segments.len() > 2, "no split happened: {:?}", v.segments);
    let _ = std::fs::remove_dir_all(dir);
}

fn db_config(db: &Path) -> Config {
    Config {
        mode: SpeedMode::Full,
        db_path: Some(db.to_path_buf()),
        ..Config::default()
    }
}

fn segment_map(v: &DownloadView) -> Vec<(u64, u64, u64)> {
    v.segments.iter().map(|s| (s.start, s.end, s.pos)).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_survive_an_engine_restart() {
    let base = server().await;
    let dir = temp_dir("persist");
    let db = dir.join("klyxdown.db");
    let engine = Engine::new(db_config(&db));
    let slow = format!("{base}/slow.bin");
    let a = engine.add(request(slow.clone(), &dir, 4)).await.unwrap();
    let b = engine.add(request(slow, &dir, 4)).await.unwrap();
    let moving = |v: &DownloadView| v.segments[1..].iter().any(|s| s.pos > s.start);
    wait_for(&engine, a, "progress on a", moving).await;
    engine.pause(a).unwrap();
    wait_for(&engine, b, "progress on b", moving).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let before = view(&engine, a);
    drop(engine);

    let engine = Engine::new(db_config(&db));
    let after = view(&engine, a);
    assert_eq!(after.state, State::Paused);
    assert_eq!((&after.name, &after.path), (&before.name, &before.path));
    assert_eq!(after.bytes, before.bytes);
    assert_eq!(segment_map(&after), segment_map(&before));
    // b was downloading, so it queued itself again.
    assert_ne!(view(&engine, b).state, State::Paused);

    let paused = AddRequest {
        start: false,
        ..request(format!("{base}/file.bin"), &dir, 1)
    };
    let c = engine.add(paused).await.unwrap();
    assert_eq!(c, b + 1, "ids continue after the stored ones");
    engine.remove(c, true).unwrap();

    engine.resume(a).unwrap();
    assert_complete(&wait_state(&engine, a, State::Done).await);
    assert_complete(&wait_state(&engine, b, State::Done).await);
    drop(engine);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_database_file_is_migrated() {
    let base = server().await;
    let dir = temp_dir("migrate");
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("empty.db");
    std::fs::write(&db, b"").unwrap();

    let engine = Engine::new(db_config(&db));
    assert!(engine.list().is_empty());
    let paused = AddRequest {
        start: false,
        ..request(format!("{base}/file.bin"), &dir, 2)
    };
    let id = engine.add(paused).await.unwrap();
    drop(engine);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
    drop(conn);
    let engine = Engine::new(db_config(&db));
    let v = view(&engine, id);
    assert_eq!((v.state, v.size), (State::Paused, Some(SIZE as u64)));
    drop(engine);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_downloads_again_from_zero() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("restart");
    let id = engine
        .add(request(format!("{base}/file.bin"), &dir, 4))
        .await
        .unwrap();
    wait_state(&engine, id, State::Done).await;

    let mut events = engine.subscribe();
    engine.restart(id).unwrap();
    let Ok(Event::Changed(v)) = events.recv().await else {
        panic!("no event for the restart");
    };
    assert_eq!((v.state, v.bytes, v.segments.len()), (State::Queued, 0, 0));
    assert_eq!(v.finished_at, None);
    assert_complete(&wait_state(&engine, id, State::Done).await);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_file_fails_instead_of_mixing_versions() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("if-range");
    let id = engine
        .add(request(format!("{base}/changing.bin"), &dir, 4))
        .await
        .unwrap();
    let v = wait_state(&engine, id, State::Failed).await;
    assert_eq!(
        v.error.as_deref(),
        Some(
            "The file changed on the server since this download started. \
             Restart it to get the current version."
        )
    );

    engine.restart(id).unwrap();
    assert_complete(&wait_state(&engine, id, State::Done).await);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_connection_is_replaced() {
    let base = server().await;
    let engine = engine();
    engine.set_slow_rule_for_tests(100, 300, 60_000);
    let dir = temp_dir("stall");
    let id = engine
        .add(request(format!("{base}/stall.bin"), &dir, 4))
        .await
        .unwrap();
    assert_eq!(view(&engine, id).segments[2].start, STALL_START as u64);
    let v = wait_state(&engine, id, State::Done).await;
    assert_complete(&v);
    assert!(engine.replacements_for_tests() >= 1, "no replacement");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn finished_file_has_mark_of_the_web() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("motw");
    let url = format!("{base}/file.bin");
    let req = AddRequest {
        headers: vec![("Referer".into(), "https://example.com/page".into())],
        ..request(url.clone(), &dir, 2)
    };
    let id = engine.add(req).await.unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    let mut stream = v.path.into_os_string();
    stream.push(":Zone.Identifier");
    let zone = std::fs::read_to_string(stream).unwrap();
    assert_eq!(
        zone,
        format!(
            "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://example.com/page\r\nHostUrl={url}\r\n"
        )
    );
    let _ = std::fs::remove_dir_all(dir);
}

async fn stays(engine: &Engine, id: u64, state: State) {
    for _ in 0..30 {
        assert_eq!(view(engine, id).state, state);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn lowering_the_limit_sends_the_newest_back_to_the_queue() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("requeue");
    let crawl = format!("{base}/crawl.bin");
    let a = engine.add(request(crawl.clone(), &dir, 2)).await.unwrap();
    let b = engine.add(request(crawl, &dir, 2)).await.unwrap();
    wait_state(&engine, b, State::Downloading).await;
    let moving = |v: &DownloadView| v.bytes > 0;
    wait_for(&engine, b, "progress on b", moving).await;

    engine.set_max_active(1);
    let back = wait_state(&engine, b, State::Queued).await;
    assert!(back.bytes > 0, "progress was kept");
    stays(&engine, b, State::Queued).await;
    assert_eq!(view(&engine, a).state, State::Downloading);

    engine.set_max_active(4);
    wait_state(&engine, b, State::Downloading).await;
    engine.pause_all();
    drop(engine);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn queues_have_their_own_limit_and_schedule() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("queues");
    let never = Schedule {
        start: 0,
        stop: 0,
        days: 0,
    };
    let night = engine.add_queue(Queue {
        id: 0,
        name: " Night ".into(),
        max_active: 1,
        stopped: false,
        schedule: Some(never),
    });
    assert_ne!(night, MAIN_QUEUE);
    assert_eq!(engine.queues()[1].name, "Night");

    let crawl = format!("{base}/crawl.bin");
    let in_night = |url: &str| AddRequest {
        queue: night,
        ..request(url.to_string(), &dir, 2)
    };
    let a = engine.add(in_night(&crawl)).await.unwrap();
    let b = engine.add(in_night(&crawl)).await.unwrap();
    // Outside its window nothing starts, while the main queue runs as usual.
    let c = engine.add(request(crawl.clone(), &dir, 2)).await.unwrap();
    wait_state(&engine, c, State::Downloading).await;
    stays(&engine, a, State::Queued).await;

    // Open the window: one at a time in this queue.
    let mut q = engine.queues()[1].clone();
    q.schedule = None;
    engine.update_queue(q.clone()).unwrap();
    wait_state(&engine, a, State::Downloading).await;
    stays(&engine, b, State::Queued).await;

    // Stopping the queue sends its running download back.
    engine.update_queue(Queue { stopped: true, ..q }).unwrap();
    wait_state(&engine, a, State::Queued).await;
    assert_eq!(view(&engine, c).state, State::Downloading);

    // Removing it moves its downloads to the main queue, where they start.
    engine.remove_queue(night).unwrap();
    assert!(engine.remove_queue(MAIN_QUEUE).is_err());
    wait_state(&engine, a, State::Downloading).await;
    let b = wait_state(&engine, b, State::Downloading).await;
    assert_eq!(b.queue, MAIN_QUEUE);
    assert!(engine.move_to_queue(b.id, night).is_err());
    engine.pause_all();
    drop(engine);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn metered_connection_holds_downloads() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("metered");
    let id = engine
        .add(request(format!("{base}/crawl.bin"), &dir, 2))
        .await
        .unwrap();
    wait_state(&engine, id, State::Downloading).await;
    engine.set_metered_hold(true);
    wait_state(&engine, id, State::Queued).await;
    assert_eq!(engine.status().hold, Some(Hold::Metered));
    stays(&engine, id, State::Queued).await;
    engine.set_metered_hold(false);
    wait_state(&engine, id, State::Downloading).await;
    assert_eq!(engine.status().hold, None);
    engine.pause_all();
    drop(engine);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_holds_downloads_until_the_period_ends() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("quota");
    engine.set_quota(Some(Quota {
        bytes: 200_000,
        hours: 1,
    }));
    let id = engine
        .add(request(format!("{base}/crawl.bin"), &dir, 2))
        .await
        .unwrap();
    wait_state(&engine, id, State::Downloading).await;
    wait_state(&engine, id, State::Queued).await;
    let status = engine.status();
    assert!(status.quota_used >= 200_000, "{status:?}");
    let Some(Hold::Quota { resumes_at }) = status.hold else {
        panic!("not held: {status:?}");
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert!(resumes_at > now + 3_500_000, "about an hour away");
    stays(&engine, id, State::Queued).await;

    engine.set_quota(None);
    wait_state(&engine, id, State::Downloading).await;
    engine.pause_all();
    drop(engine);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn mirrors_with_the_same_file_share_the_work() {
    let (base, st) = server_with_state().await;
    let engine = engine();
    let dir = temp_dir("mirrors");
    let req = AddRequest {
        mirrors: vec![
            format!("{base}/mirror.bin"),
            format!("{base}/small.bin"),   // different size: skipped
            format!("{base}/norange.bin"), // no ranges: skipped
            "https://nowhere.invalid/x.bin".into(),
        ],
        ..request(format!("{base}/file.bin"), &dir, 4)
    };
    let id = engine.add(req).await.unwrap();
    assert_eq!(view(&engine, id).mirrors, 1);
    assert_complete(&wait_state(&engine, id, State::Done).await);
    assert!(
        st.mirror_hits.load(SeqCst) >= 2,
        "the mirror served ranges too"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_is_checked_when_the_file_completes() {
    use klyxdown_engine::checksum::{Algo, Checksum, file_hash};
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("checksum");
    std::fs::create_dir_all(&dir).unwrap();
    let reference = dir.join("reference");
    std::fs::write(&reference, &*BODY).unwrap();
    let good = Checksum {
        algo: Algo::Sha256,
        hex: file_hash(&reference, Algo::Sha256).unwrap(),
    };
    let with = |checksum| AddRequest {
        checksum: Some(checksum),
        ..request(format!("{base}/file.bin"), &dir, 4)
    };
    let id = engine.add(with(good)).await.unwrap();
    assert_complete(&wait_state(&engine, id, State::Done).await);

    let bad = Checksum {
        algo: Algo::Md5,
        hex: "00000000000000000000000000000000".into(),
    };
    let id = engine.add(with(bad)).await.unwrap();
    let v = wait_state(&engine, id, State::Failed).await;
    assert!(v.error.unwrap().contains("checksum"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_link_is_refreshed_without_losing_progress() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("refresh");
    let id = engine
        .add(request(format!("{base}/gone.bin"), &dir, 4))
        .await
        .unwrap();
    let v = wait_state(&engine, id, State::Failed).await;
    assert!(v.needs_link, "403 marks the link as expired: {v:?}");

    let err = engine
        .refresh_link(id, &format!("{base}/small.bin"), Vec::new())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::DifferentFile(_)), "{err}");
    assert_eq!(view(&engine, id).state, State::Failed);

    let fresh = format!("{base}/file.bin");
    engine.refresh_link(id, &fresh, Vec::new()).await.unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    assert_complete(&v);
    assert!(!v.needs_link);
    assert_eq!(v.url, fresh);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_file_comes_out_of_a_remote_zip() {
    let base = server().await;
    let engine = engine();
    let dir = temp_dir("zip");
    let url = format!("{base}/archive.zip");
    let entries = engine.list_zip(&url, &[]).await.unwrap();
    let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.size)).collect();
    assert_eq!(names, [("readme.txt", 22), ("data/body.bin", 300_000)]);

    let pick = |entry: &str| AddRequest {
        zip_entry: Some(entry.into()),
        ..request(url.clone(), &dir, 4)
    };
    let id = engine.add(pick("data/body.bin")).await.unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    assert_eq!(v.name, "body.bin");
    assert_eq!(v.size, Some(300_000));
    assert_eq!(std::fs::read(&v.path).unwrap(), BODY[..300_000]);

    let id = engine.add(pick("readme.txt")).await.unwrap();
    let v = wait_state(&engine, id, State::Done).await;
    assert_eq!(
        std::fs::read_to_string(&v.path).unwrap(),
        "hello from the archive"
    );

    assert!(engine.add(pick("missing.txt")).await.is_err());
    assert!(
        engine
            .list_zip(&format!("{base}/file.bin"), &[])
            .await
            .is_err()
    );
    let _ = std::fs::remove_dir_all(dir);
}
