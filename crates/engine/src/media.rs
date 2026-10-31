//! Video and audio from web pages, through yt-dlp: what qualities a page offers, then the
//! download itself, with audio and video merged by ffmpeg.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering::Relaxed;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::download::Shared;
use crate::runner::{self, Finished, Job};
use crate::tools::Tools;
use crate::worker::Stop;
use crate::{Error, Result};

/// What the user picked, kept with the download so a pause and resume runs the same thing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSpec {
    pub(crate) choice: MediaChoice,
    pub(crate) dir: PathBuf,
}

/// One quality the page offers.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaChoice {
    /// "1080p", "Audio (MP3)".
    pub label: String,
    /// A yt-dlp format selection.
    pub format: String,
    /// Extract the audio to this format ("mp3", "m4a").
    pub audio: Option<String>,
    /// Estimated size, when the site says.
    pub size: Option<u64>,
    pub height: Option<u32>,
}

/// What a page offers.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub title: String,
    /// The site, as yt-dlp names it ("youtube", "vimeo", "generic").
    pub site: String,
    pub thumbnail: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    pub live: bool,
    /// Best first.
    pub choices: Vec<MediaChoice>,
}

/// Flags every run uses: no user config (predictable), one video even from a playlist link.
const BASE: &[&str] = &[
    "--ignore-config",
    "--no-playlist",
    "--no-colors",
    "--no-warnings",
];

/// Browser hand-off headers as yt-dlp flags; cookies go through a cookies file (yt-dlp
/// scopes those to their site, unlike a raw header).
fn header_args(
    headers: &[(String, String)],
    url: &str,
    cookie_file: &Path,
) -> std::io::Result<Vec<String>> {
    let mut args = Vec::new();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("user-agent") {
            args.extend(["--user-agent".into(), v.clone()]);
        } else if k.eq_ignore_ascii_case("referer") {
            args.extend(["--referer".into(), v.clone()]);
        } else if k.eq_ignore_ascii_case("cookie") {
            let host = reqwest::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default();
            write_cookies(cookie_file, &host, v)?;
            args.extend([
                "--cookies".into(),
                cookie_file.to_string_lossy().into_owned(),
            ]);
        }
    }
    Ok(args)
}

/// A cookie file path, removed when this goes away (also when a pause kills the run).
struct CookieFile(PathBuf);

impl CookieFile {
    fn new(tag: &str) -> Self {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap_or_default();
        let suffix: String = r.iter().map(|b| format!("{b:02x}")).collect();
        Self(std::env::temp_dir().join(format!("klyxdown-cookies-{tag}-{suffix}.txt")))
    }
}

impl Drop for CookieFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Netscape cookie file for one site: `a=1; b=2` as two lines.
fn write_cookies(path: &Path, host: &str, header: &str) -> std::io::Result<()> {
    let domain = host.trim_start_matches("www.");
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    for pair in header.split(';') {
        if let Some((name, value)) = pair.trim().split_once('=') {
            text.push_str(&format!(".{domain}\tTRUE\t/\tFALSE\t0\t{name}\t{value}\n"));
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(crate) async fn info(
    tools: &Tools,
    url: &str,
    headers: &[(String, String)],
) -> Result<MediaInfo> {
    let ytdlp = tools.require_ytdlp()?;
    tools.update_ytdlp().await;
    let cookies = CookieFile::new("info");
    let mut cmd = Command::new(ytdlp);
    cmd.args(BASE)
        .args(["-J", "--skip-download"])
        .args(header_args(headers, url, &cookies.0)?)
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    hide_window(&mut cmd);
    let out = tokio::time::timeout(std::time::Duration::from_secs(90), cmd.output())
        .await
        .map_err(|_| Error::Network("The site took too long to answer.".into()))??;
    drop(cookies);
    if !out.status.success() {
        return Err(Error::Network(runner::last_error(
            &String::from_utf8_lossy(&out.stderr),
            "yt-dlp",
        )));
    }
    let json: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::Network(format!("yt-dlp answered something unexpected: {e}")))?;
    Ok(MediaInfo {
        title: json["title"].as_str().unwrap_or("video").to_string(),
        site: json["extractor_key"]
            .as_str()
            .or(json["extractor"].as_str())
            .unwrap_or_default()
            .to_string(),
        thumbnail: json["thumbnail"].as_str().map(str::to_owned),
        duration: json["duration"].as_f64(),
        live: json["is_live"].as_bool().unwrap_or(false),
        choices: choices(&json),
    })
}

fn size_of(f: &Value) -> Option<u64> {
    f["filesize"].as_u64().or(f["filesize_approx"].as_u64())
}

/// Qualities from yt-dlp's format list: every video height, best first, then audio only.
pub(crate) fn choices(json: &Value) -> Vec<MediaChoice> {
    let formats = json["formats"].as_array().cloned().unwrap_or_default();
    let has = |f: &Value, key: &str| f[key].as_str().is_some_and(|c| c != "none");
    let audio_size = formats
        .iter()
        .filter(|f| has(f, "acodec") && !has(f, "vcodec"))
        .filter_map(size_of)
        .max();
    // Best size per height (with its frame rate), from the video formats.
    let mut heights: HashMap<u32, (Option<u64>, u32)> = HashMap::new();
    for f in formats.iter().filter(|f| has(f, "vcodec")) {
        let Some(h) = f["height"].as_u64().map(|h| h as u32) else {
            continue;
        };
        let fps = f["fps"].as_f64().unwrap_or(0.0).round() as u32;
        let video_only = !has(f, "acodec");
        let size = size_of(f).map(|s| {
            s + if video_only {
                audio_size.unwrap_or(0)
            } else {
                0
            }
        });
        let entry = heights.entry(h).or_insert((None, 0));
        entry.0 = entry.0.max(size);
        entry.1 = entry.1.max(fps);
    }
    let mut sorted: Vec<_> = heights.into_iter().collect();
    sorted.sort_by_key(|h| std::cmp::Reverse(h.0));
    let mut out: Vec<MediaChoice> = sorted
        .into_iter()
        .map(|(h, (size, fps))| MediaChoice {
            label: if fps > 30 {
                format!("{h}p{fps}")
            } else {
                format!("{h}p")
            },
            format: format!("bv*[height<={h}]+ba/b[height<={h}]/b"),
            audio: None,
            size,
            height: Some(h),
        })
        .collect();
    if out.is_empty() && formats.is_empty() {
        // A single file (yt-dlp's generic extractor found one link).
        out.push(MediaChoice {
            label: "Original".into(),
            format: "b".into(),
            size: size_of(json),
            ..MediaChoice::default()
        });
    }
    for (label, audio, format) in [
        ("Audio (M4A)", "m4a", "ba[ext=m4a]/ba/b"),
        ("Audio (MP3)", "mp3", "ba/b"),
    ] {
        out.push(MediaChoice {
            label: label.into(),
            format: format.into(),
            audio: Some(audio.into()),
            size: audio_size,
            height: None,
        });
    }
    out
}

/// `KDP <file>|<done>|<total>|<estimate>`, printed by our progress template.
pub(crate) fn parse_progress(line: &str) -> Option<(PathBuf, u64, Option<u64>)> {
    let rest = line.strip_prefix("KDP ")?;
    let mut parts = rest.rsplitn(4, '|');
    let (estimate, total, done, file) =
        (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let num = |s: &str| s.trim().parse::<f64>().ok().map(|n| n as u64);
    Some((
        PathBuf::from(file),
        num(done)?,
        num(total).or(num(estimate)),
    ))
}

/// What yt-dlp is doing, from its own status lines.
fn stage(line: &str) -> Option<&'static str> {
    if line.starts_with("[Merger]") {
        Some("Merging audio and video")
    } else if line.starts_with("[ExtractAudio]") {
        Some("Converting the audio")
    } else if line.starts_with("[VideoRemuxer]") || line.starts_with("[FixupM3u8]") {
        Some("Finishing the video file")
    } else {
        None
    }
}

pub(crate) async fn run(shared: &Shared, job: &Job) -> std::result::Result<Finished, Stop> {
    let stop = |msg: String| Stop {
        msg,
        expired: false,
    };
    let spec = job
        .extra
        .media
        .clone()
        .ok_or_else(|| stop("This video download lost its settings.".into()))?;
    let tools = shared.tools();
    let ytdlp = tools.require_ytdlp().map_err(|e| stop(e.to_string()))?;
    tools.update_ytdlp().await;
    std::fs::create_dir_all(&spec.dir).map_err(|e| stop(format!("File error: {e}")))?;
    let cookies = CookieFile::new(&job.id.to_string());
    let mut cmd = Command::new(ytdlp);
    cmd.args(BASE)
        .args(["--newline", "--progress", "--continue", "-N", "4"])
        .args([
            "--progress-template",
            "download:KDP %(progress.filename)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s",
        ])
        .args(["--print", "after_move:KDFILE %(filepath)s"])
        .args(["-f", &spec.choice.format])
        .args(["--merge-output-format", "mp4/mkv"])
        .args(["-o", "%(title).150B [%(id)s].%(ext)s"])
        .arg("-P")
        .arg(&spec.dir)
        .args(header_args(&job.headers, &job.url, &cookies.0).map_err(|e| stop(e.to_string()))?);
    if let Some(ffmpeg) = tools.ffmpeg() {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }
    if let Some(audio) = &spec.choice.audio {
        cmd.args(["-x", "--audio-format", audio]);
    }
    if let Some(rate) = job.rate {
        cmd.args(["--limit-rate", &rate.to_string()]);
    }
    cmd.arg("--")
        .arg(&job.url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| stop(format!("Could not start yt-dlp: {e}")))?;
    let stderr = child.stderr.take();
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                text.push_str(&l);
                text.push('\n');
            }
        }
        text
    });

    let ext = &job.ext;
    let estimate = spec.choice.size.unwrap_or(0);
    // Each format (video, then audio) is its own file with its own counter.
    let mut files: Vec<(PathBuf, u64, u64)> = Vec::new();
    let mut final_path = None;
    let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some((file, done, total)) = parse_progress(&line) {
            let mut part = file.clone().into_os_string();
            part.push(".part");
            ext.add_part(part.into());
            ext.add_part(file.clone());
            match files.iter_mut().find(|f| f.0 == file) {
                Some(f) => (f.1, f.2) = (done, total.unwrap_or(f.2)),
                None => files.push((file, done, total.unwrap_or(0))),
            }
            let done: u64 = files.iter().map(|f| f.1).sum();
            let total: u64 = files
                .iter()
                .map(|f| f.2)
                .sum::<u64>()
                .max(estimate)
                .max(done);
            ext.recv.store(done, Relaxed);
            ext.set(done, total);
            ext.set_detail(None);
        } else if let Some(path) = line.strip_prefix("KDFILE ") {
            final_path = Some(PathBuf::from(path.trim()));
        } else if let Some(what) = stage(&line) {
            ext.set_detail(Some(what));
        }
    }
    let status = child.wait().await.map_err(|e| stop(e.to_string()))?;
    let errors = errors.await.unwrap_or_default();
    drop(cookies);
    ext.set_detail(None);
    if !status.success() {
        return Err(stop(runner::last_error(&errors, "yt-dlp")));
    }
    let path = final_path.ok_or_else(|| stop("yt-dlp finished without saving a file.".into()))?;
    let size = std::fs::metadata(&path).ok().map(|m| m.len());
    Ok(Finished { path, size })
}

/// No console window flashing up on Windows for every helper run.
pub(crate) fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualities_from_formats() {
        let json = serde_json::json!({
            "title": "t",
            "formats": [
                {"format_id": "140", "vcodec": "none", "acodec": "mp4a", "filesize": 1000},
                {"format_id": "137", "vcodec": "avc1", "acodec": "none", "height": 1080, "fps": 30, "filesize": 9000},
                {"format_id": "303", "vcodec": "vp9", "acodec": "none", "height": 1080, "fps": 60, "filesize_approx": 12000},
                {"format_id": "18", "vcodec": "avc1", "acodec": "mp4a", "height": 360, "filesize": 3000},
                {"format_id": "sb0", "vcodec": "none", "acodec": "none"}
            ]
        });
        let c = choices(&json);
        let labels: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["1080p60", "360p", "Audio (M4A)", "Audio (MP3)"]);
        assert_eq!(c[0].size, Some(13_000), "video plus the best audio");
        assert_eq!(c[1].size, Some(3000), "already has audio");
        assert_eq!(c[0].format, "bv*[height<=1080]+ba/b[height<=1080]/b");
        assert_eq!(c[3].audio.as_deref(), Some("mp3"));
    }

    #[test]
    fn a_plain_file_has_one_choice() {
        let c = choices(&serde_json::json!({"title": "x", "filesize": 5}));
        assert_eq!((c[0].label.as_str(), c[0].size), ("Original", Some(5)));
    }

    #[test]
    fn progress_lines() {
        assert_eq!(
            parse_progress("KDP /dl/a|b [x].f137.mp4|1048576|NA|2097152.5"),
            Some((
                PathBuf::from("/dl/a|b [x].f137.mp4"),
                1_048_576,
                Some(2_097_152)
            ))
        );
        assert_eq!(
            parse_progress("KDP /dl/v.mp4|10|20|NA"),
            Some((PathBuf::from("/dl/v.mp4"), 10, Some(20)))
        );
        assert_eq!(parse_progress("[download] 10% of 5MiB"), None);
        assert_eq!(
            stage("[Merger] Merging formats into \"x.mp4\""),
            Some("Merging audio and video")
        );
    }

    #[test]
    fn cookies_become_a_cookie_file() {
        let path = std::env::temp_dir().join(format!("kd-cookies-{}.txt", std::process::id()));
        let args = header_args(
            &[
                ("Cookie".into(), "a=1; b=x=y".into()),
                ("Referer".into(), "https://www.site.test/watch".into()),
            ],
            "https://www.site.test/v/1",
            &path,
        )
        .unwrap();
        assert!(args.contains(&"--referer".to_string()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(".site.test\tTRUE\t/\tFALSE\t0\ta\t1\n"));
        assert!(text.contains("\tb\tx=y\n"));
        let _ = std::fs::remove_file(path);
    }
}
