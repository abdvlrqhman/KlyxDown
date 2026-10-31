//! The helper programs for video: yt-dlp (finds and downloads streams) and ffmpeg (merges and
//! converts). Downloaded on first use, with the user's consent, into the app's data folder;
//! checksums are verified against the ones each project publishes.
//!
//! Linux uses the ffmpeg from the system packages (every distribution has one); the static
//! Linux builds ship as `.tar.xz`, which would need another decompressor.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tokio::process::Command;

use crate::checksum::{self, Algo};
use crate::{Error, Result};

const YTDLP_RELEASES: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
/// How often yt-dlp updates itself; sites change their players weekly.
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    /// Version of the yt-dlp KlyxDown runs, if installed.
    pub ytdlp: Option<String>,
    pub ffmpeg: Option<String>,
    /// KlyxDown can download ffmpeg itself on this system (else: the system package).
    pub ffmpeg_downloadable: bool,
}

pub(crate) struct Tools {
    dir: PathBuf,
    /// Versions found the last time; asking means starting both programs.
    // shortcut: an ffmpeg installed by hand while KlyxDown runs shows up after a restart.
    known: std::sync::Mutex<Option<ToolStatus>>,
}

/// The yt-dlp build for this system.
fn ytdlp_asset() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "yt-dlp_arm64.exe",
        ("windows", "x86") => "yt-dlp_x86.exe",
        ("windows", _) => "yt-dlp.exe",
        ("macos", _) => "yt-dlp_macos",
        (_, "aarch64") => "yt-dlp_linux_aarch64",
        _ => "yt-dlp_linux",
    }
}

/// (archive link, checksum link, files to take from it) for systems KlyxDown fetches ffmpeg on.
fn ffmpeg_source() -> Option<Vec<(String, String, &'static [&'static str])>> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", arch) => {
            let build = if arch == "aarch64" {
                "winarm64"
            } else {
                "win64"
            };
            let base = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";
            Some(vec![(
                format!("{base}/ffmpeg-master-latest-{build}-gpl.zip"),
                format!("{base}/checksums.sha256"),
                &["ffmpeg.exe", "ffprobe.exe"],
            )])
        }
        ("macos", arch) => {
            let arch = if arch == "aarch64" { "arm64" } else { "amd64" };
            let base =
                format!("https://ffmpeg.martin-riedl.de/redirect/latest/macos/{arch}/release");
            let one = |name: &'static [&'static str]| {
                let zip = format!("{base}/{}.zip", name[0]);
                (zip.clone(), format!("{zip}.sha256"), name)
            };
            Some(vec![one(&["ffmpeg"]), one(&["ffprobe"])])
        }
        _ => None,
    }
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// A program on the PATH.
fn which(name: &str) -> Option<PathBuf> {
    let name = exe(name);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&name))
        .find(|p| p.is_file())
        .or_else(|| {
            // Apps started from the macOS Finder don't get Homebrew's PATH.
            ["/opt/homebrew/bin", "/usr/local/bin"]
                .iter()
                .map(|d| Path::new(d).join(&name))
                .find(|p| cfg!(target_os = "macos") && p.is_file())
        })
}

impl Tools {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            known: std::sync::Mutex::default(),
        }
    }

    fn forget_versions(&self) {
        *self.known.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(crate) fn ytdlp(&self) -> PathBuf {
        self.dir.join(exe("yt-dlp"))
    }

    /// Ours first, then the system's.
    pub(crate) fn ffmpeg(&self) -> Option<PathBuf> {
        let ours = self.dir.join(exe("ffmpeg"));
        if ours.is_file() {
            Some(ours)
        } else {
            which("ffmpeg")
        }
    }

    pub(crate) async fn status(&self) -> ToolStatus {
        if let Some(known) = self.known.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return known;
        }
        let ytdlp = self.ytdlp();
        let status = ToolStatus {
            ytdlp: if ytdlp.is_file() {
                version(&ytdlp, "--version").await
            } else {
                None
            },
            ffmpeg: match self.ffmpeg() {
                Some(path) => version(&path, "-version").await,
                None => None,
            },
            ffmpeg_downloadable: ffmpeg_source().is_some(),
        };
        *self.known.lock().unwrap_or_else(|e| e.into_inner()) = Some(status.clone());
        status
    }

    pub(crate) fn require_ytdlp(&self) -> Result<PathBuf> {
        let path = self.ytdlp();
        if path.is_file() {
            Ok(path)
        } else {
            Err(Error::ToolMissing("yt-dlp".into()))
        }
    }

    pub(crate) fn require_ffmpeg(&self) -> Result<PathBuf> {
        self.ffmpeg()
            .ok_or_else(|| Error::ToolMissing("ffmpeg".into()))
    }

    /// Downloads yt-dlp, and ffmpeg where KlyxDown provides it.
    pub(crate) async fn install(&self, client: &reqwest::Client) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        if !self.ytdlp().is_file() {
            let asset = ytdlp_asset();
            let sums = text(client, &format!("{YTDLP_RELEASES}/SHA2-256SUMS")).await?;
            let want = sum_for(&sums, asset)
                .ok_or_else(|| Error::Network(format!("no published checksum for {asset}")))?;
            let tmp = self.dir.join(format!("{asset}.download"));
            fetch(client, &format!("{YTDLP_RELEASES}/{asset}"), &tmp).await?;
            verify(&tmp, &want)?;
            make_executable(&tmp)?;
            tokio::fs::rename(&tmp, self.ytdlp()).await?;
        }
        let sources = if self.ffmpeg().is_none() {
            ffmpeg_source().unwrap_or_default()
        } else {
            Vec::new()
        };
        for (url, sums, files) in sources {
            let tmp = self.dir.join("ffmpeg.zip.download");
            fetch(client, &url, &tmp).await?;
            let archive = url.rsplit('/').next().unwrap_or_default();
            // Checked when the project publishes a checksum we can find; HTTPS otherwise.
            match text(client, &sums)
                .await
                .ok()
                .and_then(|s| sum_for(&s, archive).or_else(|| first_hex(&s)))
            {
                Some(want) => verify(&tmp, &want)?,
                None => tracing::warn!("no checksum published for {url}; relying on HTTPS"),
            }
            let (zip, dir) = (tmp.clone(), self.dir.clone());
            tokio::task::spawn_blocking(move || -> io::Result<()> {
                for name in files {
                    let target = dir.join(name);
                    crate::zip::extract_local(&zip, name, &target)?;
                    make_executable(&target)?;
                }
                Ok(())
            })
            .await
            .map_err(io::Error::other)??;
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        self.forget_versions();
        Ok(())
    }

    /// `yt-dlp -U` at most once a day: extractors break when sites change.
    pub(crate) async fn update_ytdlp(&self) {
        let stamp = self.dir.join("yt-dlp.updated");
        let fresh = std::fs::metadata(&stamp)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < UPDATE_EVERY);
        if fresh || !self.ytdlp().is_file() {
            return;
        }
        let _ = std::fs::write(&stamp, b"");
        let mut cmd = Command::new(self.ytdlp());
        cmd.args(["-U", "--no-colors"]).kill_on_drop(true);
        crate::media::hide_window(&mut cmd);
        match tokio::time::timeout(Duration::from_secs(120), cmd.output()).await {
            Ok(Ok(out)) if out.status.success() => tracing::info!("yt-dlp is up to date"),
            other => tracing::warn!("yt-dlp could not update itself: {other:?}"),
        }
        self.forget_versions();
    }
}

async fn version(path: &Path, flag: &str) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg(flag).kill_on_drop(true);
    crate::media::hide_window(&mut cmd);
    let out = cmd.output().await.ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?.trim();
    // "ffmpeg version 7.1 Copyright…" → "7.1"
    Some(
        first
            .strip_prefix("ffmpeg version ")
            .and_then(|v| v.split_whitespace().next())
            .unwrap_or(first)
            .to_string(),
    )
}

async fn text(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Network(crate::probe::describe(&e)))?;
    if !resp.status().is_success() {
        return Err(Error::Status(resp.status().as_u16()));
    }
    resp.text()
        .await
        .map_err(|e| Error::Network(crate::probe::describe(&e)))
}

async fn fetch(client: &reqwest::Client, url: &str, to: &Path) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Network(crate::probe::describe(&e)))?;
    if !resp.status().is_success() {
        return Err(Error::Status(resp.status().as_u16()));
    }
    let mut file = tokio::fs::File::create(to).await?;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Network(crate::probe::describe(&e)))?
    {
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    Ok(())
}

/// The hash for `name` in a `sha256sum`-style list ("<hex>  <name>" or "<hex> *<name>").
fn sum_for(list: &str, name: &str) -> Option<String> {
    list.lines().find_map(|line| {
        let (hex, file) = line.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hex.to_ascii_lowercase())
    })
}

/// A file that holds just one hash.
fn first_hex(text: &str) -> Option<String> {
    let hex = text.split_whitespace().next()?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

fn verify(path: &Path, want: &str) -> Result<()> {
    if checksum::file_hash(path, Algo::Sha256)? == want {
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    Err(Error::Network(
        "The download did not match its published checksum, so it was thrown away.".into(),
    ))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_checksum_lists() {
        let list = "aa11  yt-dlp\nBB22  yt-dlp.exe\ncc33 *yt-dlp_macos\n";
        assert_eq!(sum_for(list, "yt-dlp.exe").as_deref(), Some("bb22"));
        assert_eq!(sum_for(list, "yt-dlp_macos").as_deref(), Some("cc33"));
        assert_eq!(sum_for(list, "yt-dlp_linux"), None);
        let one = format!("{}  ffmpeg.zip\n", "a".repeat(64));
        assert_eq!(first_hex(&one), Some("a".repeat(64)));
        assert_eq!(first_hex("<html>not found</html>"), None);
    }
}
