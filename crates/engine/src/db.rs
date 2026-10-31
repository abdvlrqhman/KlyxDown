//! SQLite persistence: read once at startup, then written by a dedicated thread so the async
//! runtime never waits on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::time::Duration;

use rusqlite::{Connection, params};
use tracing::warn;

use crate::download::{Extra, part_path};
use crate::queue::{Queue, QueueId};
use crate::{Id, Kind, State};

/// One row of `downloads` plus its segments.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Record {
    pub(crate) id: Id,
    pub(crate) url: String,
    pub(crate) final_url: String,
    pub(crate) name: String,
    pub(crate) host: String,
    pub(crate) path: PathBuf,
    pub(crate) size: Option<u64>,
    pub(crate) state: State,
    pub(crate) error: Option<String>,
    pub(crate) resumable: bool,
    pub(crate) connections: u8,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) etag: Option<String>,
    pub(crate) last_modified: Option<String>,
    pub(crate) added_at: u64,
    pub(crate) finished_at: Option<u64>,
    /// `(start, end, pos)`; `end` is `u64::MAX` while the size is unknown.
    pub(crate) segments: Vec<(u64, u64, u64)>,
    pub(crate) kind: Kind,
    pub(crate) queue: QueueId,
    pub(crate) extra: Extra,
}

enum Op {
    Save(Vec<Record>),
    Delete(Id),
    SaveQueue(Queue),
    DeleteQueue(QueueId),
    Flush(SyncSender<()>),
}

/// Handle to the writer thread. It exits once this is dropped and the queue is drained.
pub(crate) struct Db {
    tx: Sender<Op>,
}

/// Index `i` upgrades `user_version` from `i` to `i + 1`. Only ever append.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE downloads (
    id INTEGER PRIMARY KEY,
    url TEXT NOT NULL,
    final_url TEXT NOT NULL,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    path TEXT NOT NULL,
    size INTEGER,
    state TEXT NOT NULL,
    error TEXT,
    resumable INTEGER NOT NULL,
    connections INTEGER NOT NULL,
    headers TEXT NOT NULL,
    etag TEXT,
    last_modified TEXT,
    added_at INTEGER NOT NULL,
    finished_at INTEGER
);
-- u64 columns are stored as i64; "end" = -1 means the size is unknown.
CREATE TABLE segments (
    download_id INTEGER NOT NULL,
    start INTEGER NOT NULL,
    "end" INTEGER NOT NULL,
    pos INTEGER NOT NULL,
    PRIMARY KEY (download_id, start)
);
"#,
    r#"
ALTER TABLE downloads ADD COLUMN kind TEXT NOT NULL DEFAULT 'direct';
ALTER TABLE downloads ADD COLUMN queue INTEGER NOT NULL DEFAULT 0;
-- JSON: mirrors, checksum, media format and other per-kind details.
ALTER TABLE downloads ADD COLUMN extra TEXT NOT NULL DEFAULT '{}';
-- schedule is JSON, NULL when the queue runs any time.
CREATE TABLE queues (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    max_active INTEGER NOT NULL,
    stopped INTEGER NOT NULL DEFAULT 0,
    schedule TEXT
);
"#,
];

impl Db {
    /// Opens (creating and migrating as needed) and loads every download, in add order,
    /// and every queue. `None`, or a database that can't be opened, keeps everything in memory.
    pub(crate) fn open(path: Option<&Path>) -> (Self, Vec<Record>, Vec<Queue>) {
        let opened = connect(path).and_then(|c| {
            let records = load(&c)?;
            let queues = load_queues(&c)?;
            Ok((c, records, queues))
        });
        let (conn, records, queues) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                warn!(
                    ?path,
                    "download database unavailable, keeping downloads in memory: {e}"
                );
                let conn = connect(None).expect("in-memory SQLite always opens");
                (conn, Vec::new(), Vec::new())
            }
        };
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("klyxdown-db".into())
            .spawn(move || write_loop(conn, rx))
            .expect("spawn the database writer thread");
        (Self { tx }, records, queues)
    }

    pub(crate) fn save(&self, records: Vec<Record>) {
        if !records.is_empty() {
            let _ = self.tx.send(Op::Save(records));
        }
    }

    pub(crate) fn delete(&self, id: Id) {
        let _ = self.tx.send(Op::Delete(id));
    }

    pub(crate) fn save_queue(&self, queue: Queue) {
        let _ = self.tx.send(Op::SaveQueue(queue));
    }

    pub(crate) fn delete_queue(&self, id: QueueId) {
        let _ = self.tx.send(Op::DeleteQueue(id));
    }

    /// Blocks until everything queued so far is written (5 s at most).
    pub(crate) fn flush(&self) {
        let (ack, done) = mpsc::sync_channel(1);
        if self.tx.send(Op::Flush(ack)).is_ok() {
            let _ = done.recv_timeout(Duration::from_secs(5));
        }
    }
}

fn connect(path: Option<&Path>) -> rusqlite::Result<Connection> {
    let mut conn = match path {
        Some(p) => {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            Connection::open(p)?
        }
        None => Connection::open_in_memory()?,
    };
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

fn load(conn: &Connection) -> rusqlite::Result<Vec<Record>> {
    let mut segments: HashMap<i64, Vec<(u64, u64, u64)>> = HashMap::new();
    let mut stmt = conn.prepare(
        r#"SELECT download_id, start, "end", pos FROM segments ORDER BY download_id, start"#,
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            (
                r.get::<_, i64>(1)? as u64,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ),
        ))
    })?;
    for row in rows {
        let (id, segment) = row?;
        segments.entry(id).or_default().push(segment);
    }

    let mut stmt = conn.prepare(
        "SELECT id, url, final_url, name, host, path, size, state, error, resumable, connections,
                headers, etag, last_modified, added_at, finished_at, kind, queue, extra
         FROM downloads ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        let id: i64 = r.get(0)?;
        let headers: String = r.get(11)?;
        let extra: String = r.get(18)?;
        Ok(Record {
            id: id as u64,
            url: r.get(1)?,
            final_url: r.get(2)?,
            name: r.get(3)?,
            host: r.get(4)?,
            path: PathBuf::from(r.get::<_, String>(5)?),
            size: r.get::<_, Option<i64>>(6)?.map(|s| s as u64),
            state: parse_state(&r.get::<_, String>(7)?),
            error: r.get(8)?,
            resumable: r.get(9)?,
            connections: r.get(10)?,
            headers: serde_json::from_str(&headers).unwrap_or_default(),
            etag: r.get(12)?,
            last_modified: r.get(13)?,
            added_at: r.get::<_, i64>(14)? as u64,
            finished_at: r.get::<_, Option<i64>>(15)?.map(|t| t as u64),
            segments: segments.remove(&id).unwrap_or_default(),
            kind: serde_json::from_value(r.get::<_, String>(16)?.into()).unwrap_or(Kind::Direct),
            queue: r.get(17)?,
            extra: serde_json::from_str(&extra).unwrap_or_default(),
        })
    })?;
    rows.collect()
}

fn load_queues(conn: &Connection) -> rusqlite::Result<Vec<Queue>> {
    let mut stmt =
        conn.prepare("SELECT id, name, max_active, stopped, schedule FROM queues ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(Queue {
            id: r.get(0)?,
            name: r.get(1)?,
            max_active: r.get::<_, i64>(2)?.max(1) as usize,
            stopped: r.get(3)?,
            schedule: r
                .get::<_, Option<String>>(4)?
                .and_then(|s| serde_json::from_str(&s).ok()),
        })
    })?;
    rows.collect()
}

fn write_loop(mut conn: Connection, rx: Receiver<Op>) {
    for op in rx {
        let written = match op {
            Op::Save(records) => {
                sync_parts(&records);
                save(&mut conn, &records)
            }
            Op::SaveQueue(q) => save_queue(&conn, &q),
            Op::DeleteQueue(id) => conn
                .execute("DELETE FROM queues WHERE id = ?1", [id])
                .map(drop),
            Op::Delete(id) => conn
                .execute("DELETE FROM segments WHERE download_id = ?1", [id as i64])
                .and_then(|_| conn.execute("DELETE FROM downloads WHERE id = ?1", [id as i64]))
                .map(drop),
            Op::Flush(ack) => {
                let _ = ack.send(());
                Ok(())
            }
        };
        if let Err(e) = written {
            warn!("could not save downloads: {e}");
        }
    }
}

fn save_queue(conn: &Connection, q: &Queue) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO queues (id, name, max_active, stopped, schedule)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            q.id,
            q.name,
            q.max_active as i64,
            q.stopped,
            q.schedule.and_then(|s| serde_json::to_string(&s).ok()),
        ],
    )
    .map(drop)
}

/// Progress is never saved before the bytes it describes are on disk: a power cut after this
/// leaves at worst some bytes downloaded twice, never zeros in a range marked done.
fn sync_parts(records: &[Record]) {
    for r in records
        .iter()
        .filter(|r| r.state != State::Done && !r.segments.is_empty())
    {
        // Write access: Windows refuses to flush a file opened read-only.
        if let Ok(file) = std::fs::OpenOptions::new()
            .write(true)
            .open(part_path(&r.path))
            && let Err(e) = file.sync_data()
        {
            warn!(id = r.id, "could not flush the partial file: {e}");
        }
    }
}

fn save(conn: &mut Connection, records: &[Record]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut row = tx.prepare_cached(
            "INSERT OR REPLACE INTO downloads (id, url, final_url, name, host, path, size, state,
                error, resumable, connections, headers, etag, last_modified, added_at, finished_at,
                kind, queue, extra)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19)",
        )?;
        let mut clear = tx.prepare_cached("DELETE FROM segments WHERE download_id = ?1")?;
        let mut segment = tx.prepare_cached(
            r#"INSERT INTO segments (download_id, start, "end", pos) VALUES (?1, ?2, ?3, ?4)"#,
        )?;
        for r in records {
            let id = r.id as i64;
            row.execute(params![
                id,
                r.url,
                r.final_url,
                r.name,
                r.host,
                r.path.to_string_lossy(),
                r.size.map(|s| s as i64),
                state_str(r.state),
                r.error,
                r.resumable,
                r.connections,
                serde_json::to_string(&r.headers).unwrap_or_else(|_| "[]".into()),
                r.etag,
                r.last_modified,
                r.added_at as i64,
                r.finished_at.map(|t| t as i64),
                serde_json::to_value(r.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned)),
                r.queue,
                serde_json::to_string(&r.extra).unwrap_or_else(|_| "{}".into()),
            ])?;
            clear.execute([id])?;
            for &(start, end, pos) in &r.segments {
                segment.execute([id, start as i64, end as i64, pos as i64])?;
            }
        }
    }
    tx.commit()
}

fn state_str(state: State) -> &'static str {
    match state {
        State::Queued => "queued",
        State::Downloading => "downloading",
        State::Paused => "paused",
        State::WaitingForNetwork => "waitingForNetwork",
        State::Failed => "failed",
        State::Done => "done",
    }
}

fn parse_state(s: &str) -> State {
    match s {
        "queued" => State::Queued,
        "downloading" => State::Downloading,
        "waitingForNetwork" => State::WaitingForNetwork,
        "failed" => State::Failed,
        "done" => State::Done,
        _ => State::Paused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut conn = connect(None).unwrap();
        let record = Record {
            id: 7,
            url: "https://x.test/a".into(),
            final_url: "https://cdn.x.test/a".into(),
            name: "a".into(),
            host: "cdn.x.test".into(),
            path: PathBuf::from("C:/dl/a"),
            size: None,
            state: State::WaitingForNetwork,
            error: Some("offline".into()),
            resumable: true,
            connections: 1,
            headers: vec![("Referer".into(), "https://x.test/".into())],
            etag: Some("\"v1\"".into()),
            last_modified: None,
            added_at: 1,
            finished_at: None,
            segments: vec![(0, u64::MAX, 12_345)],
            kind: Kind::Direct,
            queue: 3,
            extra: Extra::default(),
        };
        save(&mut conn, std::slice::from_ref(&record)).unwrap();
        save(&mut conn, std::slice::from_ref(&record)).unwrap();
        assert_eq!(load(&conn).unwrap(), vec![record]);
    }

    #[test]
    fn version_1_databases_keep_their_downloads() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute(
            "INSERT INTO downloads VALUES (1, 'u', 'u', 'a', 'h', '/a', 5, 'paused', NULL, 1, 2,
             '[]', NULL, NULL, 0, NULL)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        migrate(&mut conn).unwrap();
        let r = &load(&conn).unwrap()[0];
        assert_eq!((r.kind, r.queue, r.state), (Kind::Direct, 0, State::Paused));
        assert!(load_queues(&conn).unwrap().is_empty());
    }

    #[test]
    fn queues_round_trip() {
        let conn = connect(None).unwrap();
        let q = Queue {
            id: 2,
            name: "Night".into(),
            max_active: 3,
            stopped: false,
            schedule: Some(crate::Schedule {
                start: 60,
                stop: 420,
                days: crate::Schedule::EVERY_DAY,
            }),
        };
        save_queue(&conn, &q).unwrap();
        save_queue(&conn, &q).unwrap();
        assert_eq!(load_queues(&conn).unwrap(), vec![q]);
    }
}
