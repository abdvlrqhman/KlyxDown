//! Torrents against a real local seeder; video downloads and conversions against small
//! stand-ins for yt-dlp and ffmpeg (shell scripts, so Unix only).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use klyxdown_engine::{AddRequest, Config, DownloadView, Engine, Kind, SpeedMode, State};

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("klyxdown-run-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn engine(data: &Path) -> Engine {
    Engine::new(Config {
        mode: SpeedMode::Full,
        data_dir: Some(data.to_path_buf()),
        torrent_dht: false,
        ..Config::default()
    })
}

async fn wait(
    engine: &Engine,
    id: u64,
    what: &str,
    done: impl Fn(&DownloadView) -> bool,
) -> DownloadView {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let v = engine
            .list()
            .into_iter()
            .find(|d| d.id == id)
            .expect("download exists");
        if done(&v) {
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {v:#?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn torrent_downloads_from_a_local_seeder_and_shares() {
    use librqbit::{CreateTorrentOptions, ListenerOptions, Session, SessionOptions};

    let root = temp_dir("torrent");
    let seed_dir = root.join("seed");
    std::fs::create_dir_all(&seed_dir).unwrap();
    let body: Vec<u8> = (0..3_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8)
        .collect();
    std::fs::write(seed_dir.join("big file.bin"), &body).unwrap();

    let seeder = Session::new_with_opts(
        seed_dir.clone(),
        SessionOptions {
            dht: None,
            listen: Some(ListenerOptions {
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..ListenerOptions::default()
            }),
            disable_local_service_discovery: true,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    let (made, _) = seeder
        .create_and_serve_torrent(
            &seed_dir.join("big file.bin"),
            CreateTorrentOptions {
                name: None,
                trackers: Vec::new(),
                piece_length: Some(256 * 1024),
            },
        )
        .await
        .unwrap();
    let magnet = made.as_magnet().to_string();
    let seeder_addr = seeder.listen_addr().unwrap();

    let engine = engine(&root.join("data"));
    engine.set_torrent_peers_for_tests(vec![seeder_addr]);
    let info = engine.inspect_torrent(&magnet).await.unwrap();
    assert_eq!(info.files.len(), 1);
    assert_eq!(
        (info.files[0].path.as_str(), info.size),
        ("big file.bin", 3_000_000)
    );

    let dir = root.join("downloads");
    let id = engine
        .add(AddRequest {
            url: magnet.clone(),
            dir: dir.clone(),
            start: true,
            seed: true,
            ..AddRequest::default()
        })
        .await
        .unwrap();
    let v = wait(&engine, id, "torrent done", |v| v.state == State::Done).await;
    assert_eq!(v.kind, Kind::Torrent);
    assert_eq!(v.path, dir.join("big file.bin"));
    assert_eq!(std::fs::read(&v.path).unwrap(), body);
    assert!(v.seeding, "keeps sharing after it finishes");
    assert!(
        engine
            .add(AddRequest {
                url: magnet,
                dir,
                ..AddRequest::default()
            })
            .await
            .is_err(),
        "no duplicates"
    );

    engine.set_seeding(id, false).unwrap();
    wait(&engine, id, "sharing stops", |v| !v.seeding).await;
    // Tauri calls this from its main thread, which has no async runtime.
    let e = engine.clone();
    std::thread::spawn(move || e.remove(id, true))
        .join()
        .expect("removing a torrent must not panic")
        .unwrap();
    seeder.stop().await;
    let _ = std::fs::remove_dir_all(root);
}

/// A stand-in for yt-dlp: `-J` describes a video; a download prints our progress
/// template's lines, then writes the file and prints where it went.
#[cfg(unix)]
const FAKE_YTDLP: &str = r#"#!/bin/sh
case " $* " in
  *" --version "*) echo 2026.10.01; exit 0 ;;
  *" -U "*) exit 0 ;;
  *" -J "*) echo '{"title":"A clip","extractor_key":"Test","duration":12,"formats":[{"vcodec":"none","acodec":"opus","filesize":100},{"vcodec":"vp9","acodec":"none","height":720,"filesize":900}]}'; exit 0 ;;
esac
dir=""; prev=""
for a in "$@"; do [ "$prev" = "-P" ] && dir="$a"; prev="$a"; done
echo "KDP $dir/A clip [x].f1.webm|400|NA|900"
echo "KDP $dir/A clip [x].f1.webm|900|900|NA"
echo "KDP $dir/A clip [x].f2.webm|100|100|NA"
echo "[Merger] Merging formats into \"$dir/A clip [x].mp4\""
printf 'video' > "$dir/A clip [x].mp4"
echo "KDFILE $dir/A clip [x].mp4"
"#;

/// A stand-in for ffmpeg: prints the banner and progress, writes the last argument.
#[cfg(unix)]
const FAKE_FFMPEG: &str = r#"#!/bin/sh
[ "$1" = "-version" ] && { echo "ffmpeg version 7.1 test"; exit 0; }
echo "  Duration: 00:00:10.00, start: 0.000000, bitrate: 1 kb/s" >&2
echo "out_time_us=5000000"
for last; do :; done
printf 'converted' > "$last"
echo "out_time_us=10000000"
echo "progress=end"
"#;

#[cfg(unix)]
fn install_fakes(data: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let tools = data.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    for (name, script) in [("yt-dlp", FAKE_YTDLP), ("ffmpeg", FAKE_FFMPEG)] {
        let path = tools.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Pretend yt-dlp updated itself today.
    std::fs::write(tools.join("yt-dlp.updated"), b"").unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn video_downloads_then_converts() {
    let root = temp_dir("media");
    let data = root.join("data");
    let engine = engine(&data);
    let missing = engine
        .media_info("https://video.test/watch?v=x", &[])
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("yt-dlp"), "{missing}");

    install_fakes(&data);
    let tools = engine.tools().await;
    assert_eq!(tools.ytdlp.as_deref(), Some("2026.10.01"));
    assert_eq!(tools.ffmpeg.as_deref(), Some("7.1"));
    // Asked again (each time Settings opens): answered from memory, nothing is started.
    std::fs::remove_file(data.join("tools/ffmpeg")).unwrap();
    assert_eq!(engine.tools().await, tools);
    install_fakes(&data);

    let info = engine
        .media_info("https://video.test/watch?v=x", &[])
        .await
        .unwrap();
    assert_eq!(
        (info.title.as_str(), info.site.as_str()),
        ("A clip", "Test")
    );
    assert_eq!(info.choices[0].label, "720p");

    let dir = root.join("videos");
    let id = engine
        .add(AddRequest {
            url: "https://video.test/watch?v=x".into(),
            dir: dir.clone(),
            file_name: Some(info.title.clone()),
            start: true,
            media: Some(info.choices[0].clone()),
            convert: Some("mp3".into()),
            ..AddRequest::default()
        })
        .await
        .unwrap();
    let v = wait(&engine, id, "video done", |v| v.state == State::Done).await;
    assert_eq!(v.kind, Kind::Media);
    assert_eq!(v.path, dir.join("A clip [x].mp4"));
    assert_eq!(v.name, "A clip [x].mp4");
    assert_eq!(v.size, Some(5));

    // "Convert after download" queued an MP3 conversion of it.
    let conv = wait(&engine, id + 1, "conversion done", |v| {
        v.state == State::Done
    })
    .await;
    assert_eq!(conv.kind, Kind::Convert);
    assert_eq!(conv.path, dir.join("A clip [x].mp3"));
    assert_eq!(std::fs::read_to_string(&conv.path).unwrap(), "converted");

    // And by hand, from a finished download.
    let again = engine.convert(id, "mkv-copy").unwrap();
    let conv = wait(&engine, again, "second conversion", |v| {
        v.state == State::Done
    })
    .await;
    assert_eq!(conv.path, dir.join("A clip [x].mkv"));
    assert!(engine.convert(id, "nonsense").is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn failing_tool_fails_the_download_with_its_message() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_dir("media-fail");
    let data = root.join("data");
    install_fakes(&data);
    let tool = data.join("tools/yt-dlp");
    std::fs::write(
        &tool,
        "#!/bin/sh\necho 'ERROR: [test] x: This video is private' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let engine = engine(&data);
    let id = engine
        .add(AddRequest {
            url: "https://video.test/private".into(),
            dir: root.join("videos"),
            start: true,
            media: Some(Default::default()),
            ..AddRequest::default()
        })
        .await
        .unwrap();
    let v = wait(&engine, id, "failure", |v| v.state == State::Failed).await;
    assert_eq!(v.error.as_deref(), Some("[test] x: This video is private"));
    let _ = std::fs::remove_dir_all(root);
}
