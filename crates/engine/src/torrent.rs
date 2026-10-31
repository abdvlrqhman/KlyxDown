//! BitTorrent through librqbit: magnets and `.torrent` files, picking files, sharing
//! (seeding) when done, and streaming for previews.
//!
//! One librqbit session for the whole engine, started on the first torrent. Torrents are kept
//! in KlyxDown's own database (with their `.torrent` bytes), not librqbit's: on a restart they
//! are added again and librqbit checks what is already on disk.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use librqbit::api::TorrentIdOrHash;
use librqbit::dht::{DhtPersistenceConfig, Id20};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerOptions, Session,
    SessionOptions, TorrentStatsState,
};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use crate::download::Shared;
use crate::runner::{Finished, Job};
use crate::worker::Stop;
use crate::{Error, Result};

/// Saved with the download.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TorrentSpec {
    /// Hex.
    pub(crate) info_hash: String,
    /// The `.torrent` file, base64, so a restart never waits for the swarm again.
    pub(crate) bytes: String,
    /// Indexes of the files to download; `None` is all.
    pub(crate) files: Option<Vec<usize>>,
    pub(crate) dir: PathBuf,
}

/// What a torrent holds, for the file picker.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentInfo {
    pub name: String,
    pub info_hash: String,
    pub size: u64,
    pub files: Vec<TorrentFile>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    pub index: usize,
    /// Inside the torrent's folder, with `/` separators.
    pub path: String,
    pub size: u64,
}

pub(crate) struct Torrents {
    session: OnceCell<Arc<Session>>,
    data_dir: PathBuf,
    /// Off in tests, which connect the peers directly.
    dht: bool,
    /// Peers to try first (tests).
    pub(crate) peers: Mutex<Vec<SocketAddr>>,
    /// `.torrent` bytes of inspected torrents, so adding one doesn't ask the swarm again.
    inspected: Mutex<Vec<(String, bytes::Bytes)>>,
    rate: Mutex<Option<u64>>,
}

pub(crate) fn is_torrent_source(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("magnet:")
        || lower
            .split(['?', '#'])
            .next()
            .is_some_and(|p| p.ends_with(".torrent"))
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::Network(e.to_string())
}

impl Torrents {
    pub(crate) fn new(data_dir: PathBuf, dht: bool) -> Self {
        Self {
            session: OnceCell::new(),
            data_dir,
            dht,
            peers: Mutex::default(),
            inspected: Mutex::default(),
            rate: Mutex::default(),
        }
    }

    pub(crate) async fn session(&self) -> Result<Arc<Session>> {
        self.session
            .get_or_try_init(|| async {
                let opts = SessionOptions {
                    dht: self.dht.then(|| DhtSessionConfig {
                        persistence: Some(DhtPersistenceConfig {
                            config_filename: Some(self.data_dir.join("dht.json")),
                            ..DhtPersistenceConfig::default()
                        }),
                        ..DhtSessionConfig::default()
                    }),
                    listen: Some(ListenerOptions {
                        // Lets other people reach us for sharing on home routers.
                        enable_upnp_port_forwarding: self.dht,
                        ..ListenerOptions::default()
                    }),
                    disable_local_service_discovery: !self.dht,
                    client_name_and_version: Some(
                        concat!("KlyxDown ", env!("CARGO_PKG_VERSION")).into(),
                    ),
                    ..SessionOptions::default()
                };
                let session = Session::new_with_opts(self.data_dir.join("torrents"), opts)
                    .await
                    .map_err(err)?;
                apply_rate(
                    &session,
                    *self.rate.lock().unwrap_or_else(|e| e.into_inner()),
                );
                Ok(session)
            })
            .await
            .cloned()
    }

    /// The speed limit and Snail mode apply to torrents too.
    pub(crate) fn set_rate(&self, rate: Option<u64>) {
        *self.rate.lock().unwrap_or_else(|e| e.into_inner()) = rate;
        if let Some(s) = self.session.get() {
            apply_rate(s, rate);
        }
    }

    /// Reads a magnet (asking the swarm for its file list), a `.torrent` link or a `.torrent`
    /// file on disk.
    pub(crate) async fn inspect(&self, source: &str) -> Result<TorrentInfo> {
        let add = if source.starts_with("magnet:") || source.starts_with("http") {
            AddTorrent::from_url(source.to_string())
        } else {
            AddTorrent::from_bytes(tokio::fs::read(source).await?)
        };
        let session = self.session().await?;
        let opts = AddTorrentOptions {
            list_only: true,
            initial_peers: Some(self.peers.lock().unwrap_or_else(|e| e.into_inner()).clone()),
            ..AddTorrentOptions::default()
        };
        let listed = tokio::time::timeout(
            Duration::from_secs(120),
            session.add_torrent(add, Some(opts)),
        )
        .await
        .map_err(|_| {
            Error::Network(
                "No one shared this torrent's file list within two minutes. Try again later."
                    .into(),
            )
        })?
        .map_err(err)?;
        let AddTorrentResponse::ListOnly(list) = listed else {
            return Err(Error::Network(
                "That torrent is already in the list.".into(),
            ));
        };
        let files: Vec<TorrentFile> = list
            .info
            .iter_file_details()
            .enumerate()
            .filter(|(_, f)| !f.attrs().padding)
            .map(|(index, f)| TorrentFile {
                index,
                path: f.filename.to_pathbuf().to_string_lossy().replace('\\', "/"),
                size: f.len,
            })
            .collect();
        let info = TorrentInfo {
            name: list
                .info
                .name()
                .map(|n| n.into_owned())
                .unwrap_or_else(|| "torrent".into()),
            info_hash: list.info_hash.as_string(),
            size: files.iter().map(|f| f.size).sum(),
            files,
        };
        let mut seen = self.inspected.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|(h, _)| *h != info.info_hash);
        seen.push((info.info_hash.clone(), list.torrent_bytes));
        if seen.len() > 32 {
            seen.remove(0);
        }
        Ok(info)
    }

    /// The `.torrent` bytes of an inspected torrent.
    pub(crate) fn bytes_of(&self, info_hash: &str) -> Option<bytes::Bytes> {
        self.inspected
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(h, _)| h == info_hash)
            .map(|(_, b)| b.clone())
    }

    /// Takes a torrent out of the session; `delete_files` removes what it downloaded.
    pub(crate) async fn forget(&self, info_hash: &str, delete_files: bool) {
        let (Some(session), Ok(hash)) = (self.session.get(), info_hash.parse::<Id20>()) else {
            return;
        };
        if let Err(e) = session
            .delete(TorrentIdOrHash::Hash(hash), delete_files)
            .await
        {
            tracing::debug!("torrent not removed: {e}");
        }
    }

    /// The biggest picked file (the film, not the subtitles) as a stream that fetches pieces
    /// in playing order: (name, size, stream).
    pub(crate) async fn stream_main(
        &self,
        info_hash: &str,
        picked: Option<&[usize]>,
    ) -> Result<(String, u64, Box<dyn crate::ReadSeek>)> {
        let session = self.session().await?;
        let hash = info_hash.parse::<Id20>().map_err(err)?;
        let handle = session
            .get(TorrentIdOrHash::Hash(hash))
            .ok_or_else(|| Error::Network("Start the torrent to preview it.".into()))?;
        let (index, name, len) = handle
            .with_metadata(|m| {
                m.file_infos
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| picked.is_none_or(|p| p.contains(i)))
                    .max_by_key(|(_, f)| f.len)
                    .map(|(i, f)| (i, f.relative_filename.to_string_lossy().into_owned(), f.len))
            })
            .map_err(err)?
            .ok_or_else(|| Error::Network("The torrent has no files to preview.".into()))?;
        let stream = handle.stream(index).await.map_err(err)?;
        Ok((name, len, Box::new(stream)))
    }
}

fn apply_rate(session: &Session, rate: Option<u64>) {
    let bps = rate.and_then(|r| std::num::NonZeroU32::new(r.min(u64::from(u32::MAX)) as u32));
    session.ratelimits.set_download_bps(bps);
}

pub(crate) fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Pauses the torrent when its runner stops (pause, remove, shutdown).
struct PauseOnDrop(Arc<Session>, Arc<librqbit::ManagedTorrent>);

impl Drop for PauseOnDrop {
    fn drop(&mut self) {
        let (session, handle) = (self.0.clone(), self.1.clone());
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = session.pause(&handle).await;
            });
        }
    }
}

fn stop(msg: impl std::fmt::Display) -> Stop {
    Stop {
        msg: msg.to_string(),
        expired: false,
    }
}

pub(crate) async fn run(shared: &Shared, job: &Job) -> std::result::Result<Finished, Stop> {
    let spec = job
        .extra
        .torrent
        .clone()
        .ok_or_else(|| stop("This torrent lost its settings."))?;
    let torrents = shared.torrents();
    let session = torrents.session().await.map_err(stop)?;
    let hash = spec.info_hash.parse::<Id20>().map_err(stop)?;
    let handle = match session.get(TorrentIdOrHash::Hash(hash)) {
        Some(h) => {
            if h.is_paused() {
                session.unpause(&h).await.map_err(stop)?;
            }
            h
        }
        None => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&spec.bytes)
                .map_err(stop)?;
            let opts = AddTorrentOptions {
                only_files: spec.files.clone(),
                output_folder: Some(spec.dir.to_string_lossy().into_owned()),
                // Our own files from before a restart: check and continue them.
                overwrite: true,
                initial_peers: Some(
                    torrents
                        .peers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone(),
                ),
                ..AddTorrentOptions::default()
            };
            session
                .add_torrent(AddTorrent::from_bytes(bytes), Some(opts))
                .await
                .map_err(stop)?
                .into_handle()
                .ok_or_else(|| stop("The torrent could not be started."))?
        }
    };
    let _pause = PauseOnDrop(session.clone(), handle.clone());
    let ext = &job.ext;
    let uploaded_before = job.extra.uploaded;
    let mut reported = job.finished;
    loop {
        let st = handle.stats();
        if let Some(e) = &st.error {
            return Err(stop(e));
        }
        let live = st.live.as_ref();
        ext.set(st.progress_bytes, st.total_bytes);
        ext.recv
            .store(live.map_or(0, |l| l.snapshot.fetched_bytes), Relaxed);
        ext.sent.store(uploaded_before + st.uploaded_bytes, Relaxed);
        let peers = live.map_or(0, |l| l.snapshot.peer_stats.live);
        ext.peers.store(u64::from(peers), Relaxed);
        ext.set_detail(match st.state {
            TorrentStatsState::Initializing { .. } => Some("Checking what is already downloaded"),
            TorrentStatsState::Live if !st.finished && peers == 0 => {
                Some("Looking for people sharing it")
            }
            _ => None,
        });
        if st.finished && !reported {
            reported = true;
            let done = Finished {
                path: job.path.clone(),
                size: Some(st.total_bytes),
            };
            if !job.extra.seed {
                return Ok(done);
            }
            shared.runner_done(job.id, job.run, Ok(done), true);
        }
        if reported && !shared.keep_seeding(job.id) {
            return Ok(Finished {
                path: job.path.clone(),
                size: None,
            });
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// (librqbit's output folder, what the list shows as the download's path): a torrent with
/// several files gets a folder named after it; a single file goes straight into `dir`.
pub(crate) fn target(
    dir: &Path,
    info: &TorrentInfo,
    files: Option<&[usize]>,
) -> (PathBuf, PathBuf) {
    let picked: Vec<&TorrentFile> = info
        .files
        .iter()
        .filter(|f| files.is_none_or(|ids| ids.contains(&f.index)))
        .collect();
    match picked.as_slice() {
        [one] if info.files.len() == 1 => (dir.to_path_buf(), dir.join(&one.path)),
        _ => {
            let folder = dir.join(crate::names::sanitize(&info.name));
            (folder.clone(), folder)
        }
    }
}
