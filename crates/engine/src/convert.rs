//! Converting finished files with ffmpeg (to MP4, MP3, …), as jobs in the download list.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering::Relaxed;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::download::{Download, Extra, Shared};
use crate::queue::QueueId;
use crate::runner::{self, Finished, Job};
use crate::worker::Stop;
use crate::{AddRequest, Id, Kind, names};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConvertSpec {
    pub(crate) input: PathBuf,
    pub(crate) preset: String,
}

/// (preset, output extension, ffmpeg arguments between input and output).
const PRESETS: &[(&str, &str, &[&str])] = &[
    // Re-encode to the most compatible video.
    (
        "mp4",
        "mp4",
        &[
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "21",
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-movflags",
            "+faststart",
        ],
    ),
    // Same streams in an MP4 box: seconds, no quality loss (TS and MKV to MP4).
    (
        "mp4-copy",
        "mp4",
        &["-map", "0", "-c", "copy", "-movflags", "+faststart"],
    ),
    ("mkv-copy", "mkv", &["-map", "0", "-c", "copy"]),
    ("mp3", "mp3", &["-vn", "-c:a", "libmp3lame", "-q:a", "2"]),
    ("m4a", "m4a", &["-vn", "-c:a", "aac", "-b:a", "192k"]),
    (
        "webm",
        "webm",
        &[
            "-c:v",
            "libvpx-vp9",
            "-crf",
            "32",
            "-b:v",
            "0",
            "-c:a",
            "libopus",
        ],
    ),
];

pub fn presets() -> Vec<&'static str> {
    PRESETS.iter().map(|p| p.0).collect()
}

fn preset(name: &str) -> Option<&'static (&'static str, &'static str, &'static [&'static str])> {
    PRESETS.iter().find(|p| p.0 == name)
}

pub(crate) fn is_preset(name: &str) -> bool {
    preset(name).is_some()
}

/// Next to the input, with the new extension; " (converted)" when it would be the same name.
pub(crate) fn output_path(input: &Path, preset_name: &str) -> PathBuf {
    let ext = preset(preset_name).map_or("mp4", |p| p.1);
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "converted".into());
    let dir = input.parent().unwrap_or(Path::new(""));
    let name = if input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
    {
        format!("{stem} (converted).{ext}")
    } else {
        format!("{stem}.{ext}")
    };
    (0..1000)
        .map(|n| dir.join(names::numbered(&name, n)))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(name))
}

/// A conversion of `input`, ready to queue.
pub(crate) fn job(id: Id, input: PathBuf, preset_name: &str, queue: QueueId) -> Download {
    let output = output_path(&input, preset_name);
    let req = AddRequest {
        url: input.to_string_lossy().into_owned(),
        start: true,
        queue,
        ..AddRequest::default()
    };
    Download::new_job(
        id,
        Kind::Convert,
        req,
        output
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        output,
        None,
        Extra {
            job: Some(ConvertSpec {
                input,
                preset: preset_name.into(),
            }),
            ..Extra::default()
        },
    )
}

/// `Duration: 00:01:02.50` in ffmpeg's banner, in milliseconds.
fn duration_ms(line: &str) -> Option<u64> {
    let rest = line.trim().strip_prefix("Duration: ")?;
    clock_ms(rest.split(',').next()?)
}

fn clock_ms(text: &str) -> Option<u64> {
    let mut parts = text.trim().split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    let secs =
        h.parse::<f64>().ok()? * 3600.0 + m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?;
    Some((secs * 1000.0) as u64)
}

pub(crate) async fn run(shared: &Shared, job: &Job) -> std::result::Result<Finished, Stop> {
    let stop = |msg: String| Stop {
        msg,
        expired: false,
    };
    let spec = job
        .extra
        .job
        .clone()
        .ok_or_else(|| stop("This conversion lost its settings.".into()))?;
    let (_, _, args) =
        preset(&spec.preset).ok_or_else(|| stop(format!("Unknown conversion: {}", spec.preset)))?;
    let ffmpeg = shared
        .tools()
        .require_ffmpeg()
        .map_err(|e| stop(e.to_string()))?;
    if !spec.input.is_file() {
        return Err(stop("The file to convert is gone.".into()));
    }
    // Written beside the target and renamed at the end, so a stopped conversion leaves no
    // half file with the real name. ffmpeg picks the format from the extension, so keep it.
    let ext = job
        .path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = job.path.with_extension(format!("kdpart.{ext}"));
    job.ext.add_part(tmp.clone());
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-y", "-i"])
        .arg(&spec.input)
        .args(*args)
        .args(["-progress", "pipe:1", "-nostats"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::media::hide_window(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| stop(format!("Could not start ffmpeg: {e}")))?;
    let ext = job.ext.clone();
    let stderr = child.stderr.take().expect("piped");
    // The banner on stderr has the duration; keep its tail for the error message.
    let banner = tokio::spawn(async move {
        let mut tail = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if let Some(ms) = duration_ms(&l) {
                ext.total.store(ms, Relaxed);
            }
            tail.push_str(&l);
            tail.push('\n');
            if tail.len() > 8192 {
                tail.drain(..4096);
            }
        }
        tail
    });
    job.ext.set_detail(Some("Converting"));
    let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(us) = line
            .strip_prefix("out_time_us=")
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            job.ext.done.store(us / 1000, Relaxed);
        }
    }
    let status = child.wait().await.map_err(|e| stop(e.to_string()))?;
    let tail = banner.await.unwrap_or_default();
    job.ext.set_detail(None);
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(stop(runner::last_error(&tail, "ffmpeg")));
    }
    std::fs::rename(&tmp, &job.path).map_err(|e| stop(format!("File error: {e}")))?;
    let size = std::fs::metadata(&job.path).ok().map(|m| m.len());
    Ok(Finished {
        path: job.path.clone(),
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffmpeg_times() {
        assert_eq!(
            duration_ms("  Duration: 00:01:02.50, start: 0.000000, bitrate: 1 kb/s"),
            Some(62_500)
        );
        assert_eq!(clock_ms("01:00:00.00"), Some(3_600_000));
        assert_eq!(duration_ms("Stream #0:0: Video"), None);
    }

    #[test]
    fn outputs_sit_next_to_the_input() {
        let dir = std::env::temp_dir().join(format!("kd-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            output_path(&dir.join("clip.ts"), "mp4-copy"),
            dir.join("clip.mp4")
        );
        assert_eq!(
            output_path(&dir.join("song.mp3"), "mp3"),
            dir.join("song (converted).mp3")
        );
        std::fs::write(dir.join("clip.mp4"), b"").unwrap();
        assert_eq!(
            output_path(&dir.join("clip.ts"), "mp4"),
            dir.join("clip (1).mp4")
        );
        assert!(is_preset("m4a") && !is_preset("exe"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
