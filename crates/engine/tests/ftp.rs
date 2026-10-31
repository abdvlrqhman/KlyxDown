//! FTP downloads against a small in-process FTP server (plain FTP, passive mode).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

use klyxdown_engine::{AddRequest, Config, Engine, SpeedMode, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const SIZE: usize = 3 * 1024 * 1024 + 777;

fn body() -> Vec<u8> {
    (0..SIZE as u64)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

#[derive(Default)]
struct Stats {
    /// RETR commands that started past byte 0 (resumed segments).
    resumed: AtomicUsize,
    sessions: AtomicUsize,
}

/// Answers just enough of RFC 959 (plus SIZE, MDTM, REST, EPSV, MLSD) for the engine.
async fn ftp_server(body: Arc<Vec<u8>>) -> (String, Arc<Stats>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stats = Arc::new(Stats::default());
    let st = stats.clone();
    tokio::spawn(async move {
        loop {
            let (conn, _) = listener.accept().await.unwrap();
            st.sessions.fetch_add(1, SeqCst);
            tokio::spawn(session(conn, body.clone(), st.clone()));
        }
    });
    (format!("ftp://127.0.0.1:{}", addr.port()), stats)
}

async fn say(w: &mut tokio::net::tcp::OwnedWriteHalf, text: String) -> std::io::Result<()> {
    w.write_all(format!("{text}\r\n").as_bytes()).await
}

async fn session(conn: TcpStream, body: Arc<Vec<u8>>, stats: Arc<Stats>) {
    let (read, mut write) = conn.into_split();
    let mut lines = BufReader::new(read).lines();
    let mut passive: Option<TcpListener> = None;
    let mut rest = 0usize;
    if say(&mut write, "220 test server".into()).await.is_err() {
        return;
    }
    while let Ok(Some(line)) = lines.next_line().await {
        let (cmd, arg) = line.split_once(' ').unwrap_or((&line, ""));
        let reply = match cmd.to_ascii_uppercase().as_str() {
            "USER" => "331 password please".into(),
            "PASS" => "230 signed in".into(),
            "TYPE" => "200 binary".into(),
            "PWD" => "257 \"/\"".into(),
            "FEAT" => "211-Features\r\n SIZE\r\n MDTM\r\n REST STREAM\r\n MLSD\r\n211 End".into(),
            "SIZE" if arg == "/pub/file.bin" => format!("213 {}", body.len()),
            "MDTM" if arg == "/pub/file.bin" => "213 20260102030405".into(),
            "SIZE" | "MDTM" => "550 no such file".into(),
            "REST" => {
                rest = arg.parse().unwrap_or(0);
                format!("350 restarting at {rest}")
            }
            "PASV" | "EPSV" => {
                let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let port = l.local_addr().unwrap().port();
                passive = Some(l);
                if cmd.eq_ignore_ascii_case("EPSV") {
                    format!("229 Entering Extended Passive Mode (|||{port}|)")
                } else {
                    format!(
                        "227 Entering Passive Mode (127,0,0,1,{},{})",
                        port >> 8,
                        port & 0xff
                    )
                }
            }
            "RETR" | "MLSD" | "LIST" => {
                let Some(l) = passive.take() else {
                    let _ = say(&mut write, "425 use PASV first".into()).await;
                    continue;
                };
                let data: Vec<u8> = if cmd.eq_ignore_ascii_case("RETR") {
                    if arg != "/pub/file.bin" {
                        let _ = say(&mut write, "550 no such file".into()).await;
                        continue;
                    }
                    if rest > 0 {
                        stats.resumed.fetch_add(1, SeqCst);
                    }
                    body[rest.min(body.len())..].to_vec()
                } else {
                    format!(
                        "type=dir;modify=20260101000000; sub folder\r\n\
                         type=file;size={};modify=20260102030405; file.bin\r\n",
                        body.len()
                    )
                    .into_bytes()
                };
                rest = 0;
                if say(&mut write, "150 opening data connection".into())
                    .await
                    .is_err()
                {
                    return;
                }
                let (mut data_conn, _) = l.accept().await.unwrap();
                // The client hangs up mid-file when its segment is complete.
                for chunk in data.chunks(32 * 1024) {
                    if data_conn.write_all(chunk).await.is_err() {
                        break;
                    }
                }
                drop(data_conn);
                "226 done".into()
            }
            "QUIT" => {
                let _ = say(&mut write, "221 bye".into()).await;
                return;
            }
            _ => "502 not implemented".into(),
        };
        if say(&mut write, reply).await.is_err() {
            return;
        }
    }
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("klyxdown-ftp-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn wait_done(engine: &Engine, id: u64) -> klyxdown_engine::DownloadView {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let v = engine.list().into_iter().find(|d| d.id == id).unwrap();
        if matches!(v.state, State::Done | State::Failed) {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out: {v:#?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ftp_file_downloads_in_segments() {
    let body = Arc::new(body());
    let (base, stats) = ftp_server(body.clone()).await;
    let engine = Engine::new(Config {
        mode: SpeedMode::Full,
        ..Config::default()
    });
    engine.set_retry_base_for_tests(10);
    let url = format!("{base}/pub/file.bin");

    let probe = engine.probe(&url).await.unwrap();
    assert_eq!(probe.name, "file.bin");
    assert_eq!(probe.size, Some(SIZE as u64));
    assert!(probe.resumable);

    let dir = temp_dir("segments");
    let id = engine
        .add(AddRequest {
            url,
            dir: dir.clone(),
            connections: 3,
            start: true,
            ..AddRequest::default()
        })
        .await
        .unwrap();
    let v = wait_done(&engine, id).await;
    assert_eq!(v.state, State::Done, "{:?}", v.error);
    assert_eq!(std::fs::read(&v.path).unwrap(), *body);
    assert!(
        stats.resumed.load(SeqCst) >= 2,
        "segments resumed with REST"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_ftp_file_is_refused_and_folders_list() {
    let (base, _) = ftp_server(Arc::new(body())).await;
    let engine = Engine::new(Config::default());
    let err = engine
        .probe(&format!("{base}/pub/nothing.bin"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("404"), "{err}");

    let entries = engine.list_folder(&format!("{base}/pub/")).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].dir && entries[0].name == "sub folder");
    assert!(entries[0].url.ends_with("/pub/sub%20folder/"));
    assert_eq!(entries[1].size, Some(SIZE as u64));
    assert!(entries[1].url.ends_with("/pub/file.bin"));
}
