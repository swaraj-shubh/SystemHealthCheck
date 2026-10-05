//! SQLite persistence: settings, scan history and benchmark/stress results.
//! Schema changes go through `MIGRATIONS` keyed by `PRAGMA user_version`.

use crate::diagnostics::Snapshot;
use crate::scoring::HealthReport;
use crate::settings::Settings;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub type DbResult<T> = Result<T, String>;

fn e(err: impl std::fmt::Display) -> String {
    err.to_string()
}

const MIGRATIONS: &[&str] = &["
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE scans (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        taken_at INTEGER NOT NULL,
        kind TEXT NOT NULL,
        score INTEGER,
        state TEXT NOT NULL,
        health TEXT NOT NULL,
        data TEXT NOT NULL
    );
    CREATE INDEX scans_taken_at ON scans(taken_at);
    CREATE TABLE benchmarks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        taken_at INTEGER NOT NULL,
        kind TEXT NOT NULL,
        summary TEXT NOT NULL,
        data TEXT NOT NULL
    );
    CREATE INDEX benchmarks_kind ON benchmarks(kind, taken_at);
"];

/// Default database path: `$SYSTEMHEALTHCHECK_DB` or `$XDG_DATA_HOME/systemhealthcheck/systemhealthcheck.db`.
pub fn default_path() -> PathBuf {
    if let Ok(p) = std::env::var("SYSTEMHEALTHCHECK_DB") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/share"));
    let path = base.join("systemhealthcheck").join("systemhealthcheck.db");
    // Carry over history from the app's previous name.
    let old = base.join("systempulse").join("systempulse.db");
    if !path.exists() && old.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::rename(&old, &path);
    }
    path
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRow {
    pub id: i64,
    pub taken_at: i64,
    pub kind: String,
    pub score: Option<u8>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRow {
    pub id: i64,
    pub taken_at: i64,
    pub kind: String,
    /// Small JSON with headline numbers (events/s, threads, peak temp...).
    pub summary: serde_json::Value,
    /// Full record (raw output etc.).
    pub data: serde_json::Value,
}

pub struct Db {
    conn: Connection,
    pub path: PathBuf,
}

impl Db {
    pub fn open(path: &Path) -> DbResult<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(e)?;
        }
        Self::init(Connection::open(path).map_err(e)?, path.to_path_buf())
    }

    pub fn open_in_memory() -> DbResult<Db> {
        Self::init(Connection::open_in_memory().map_err(e)?, PathBuf::from(":memory:"))
    }

    fn init(conn: Connection, path: PathBuf) -> DbResult<Db> {
        conn.pragma_update(None, "journal_mode", "WAL").map_err(e)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(e)?;
        for (i, m) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
            conn.execute_batch(m).map_err(e)?;
            conn.pragma_update(None, "user_version", i as i64 + 1).map_err(e)?;
        }
        Ok(Db { conn, path })
    }

    // -- settings ---------------------------------------------------------

    pub fn load_settings(&self) -> Settings {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = 'settings'", [], |r| r.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> DbResult<()> {
        self.set_value("settings", &serde_json::to_string(s).map_err(e)?)
    }

    pub fn get_value(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_value(&self, key: &str, value: &str) -> DbResult<()> {
        self.conn
            .execute("INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])
            .map(|_| ())
            .map_err(e)
    }

    // -- scans --------------------------------------------------------------

    pub fn save_scan(&self, snap: &Snapshot, health: &HealthReport) -> DbResult<i64> {
        self.conn
            .execute(
                "INSERT INTO scans(taken_at, kind, score, state, health, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    snap.taken_at,
                    snap.kind.label(),
                    health.overall,
                    health.state.label(),
                    serde_json::to_string(health).map_err(e)?,
                    serde_json::to_string(snap).map_err(e)?
                ],
            )
            .map_err(e)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_scans(&self) -> DbResult<Vec<ScanRow>> {
        let mut st = self.conn.prepare("SELECT id, taken_at, kind, score, state FROM scans ORDER BY taken_at DESC, id DESC").map_err(e)?;
        let rows = st
            .query_map([], |r| Ok(ScanRow { id: r.get(0)?, taken_at: r.get(1)?, kind: r.get(2)?, score: r.get(3)?, state: r.get(4)? }))
            .map_err(e)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(e)
    }

    pub fn load_scan(&self, id: i64) -> DbResult<(Snapshot, HealthReport)> {
        let (data, health): (String, String) =
            self.conn.query_row("SELECT data, health FROM scans WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?;
        Ok((serde_json::from_str(&data).map_err(e)?, serde_json::from_str(&health).map_err(e)?))
    }

    pub fn delete_scan(&self, id: i64) -> DbResult<()> {
        self.conn.execute("DELETE FROM scans WHERE id = ?1", [id]).map(|_| ()).map_err(e)
    }

    /// Delete history older than `days`. Returns rows removed.
    pub fn prune(&self, days: u32, now: i64) -> DbResult<usize> {
        let cutoff = now - i64::from(days) * 86_400;
        let a = self.conn.execute("DELETE FROM scans WHERE taken_at < ?1", [cutoff]).map_err(e)?;
        let b = self.conn.execute("DELETE FROM benchmarks WHERE taken_at < ?1", [cutoff]).map_err(e)?;
        Ok(a + b)
    }

    // -- benchmarks -----------------------------------------------------------

    pub fn save_bench(&self, taken_at: i64, kind: &str, summary: &serde_json::Value, data: &serde_json::Value) -> DbResult<i64> {
        self.conn
            .execute("INSERT INTO benchmarks(taken_at, kind, summary, data) VALUES (?1, ?2, ?3, ?4)", params![taken_at, kind, summary.to_string(), data.to_string()])
            .map_err(e)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_bench(&self, kind: &str) -> DbResult<Vec<BenchRow>> {
        let mut st = self.conn.prepare("SELECT id, taken_at, kind, summary, data FROM benchmarks WHERE kind = ?1 ORDER BY taken_at DESC, id DESC").map_err(e)?;
        let rows = st
            .query_map([kind], |r| {
                let s: String = r.get(3)?;
                let d: String = r.get(4)?;
                Ok(BenchRow {
                    id: r.get(0)?,
                    taken_at: r.get(1)?,
                    kind: r.get(2)?,
                    summary: serde_json::from_str(&s).unwrap_or_default(),
                    data: serde_json::from_str(&d).unwrap_or_default(),
                })
            })
            .map_err(e)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(e)
    }

    pub fn delete_bench(&self, id: i64) -> DbResult<()> {
        self.conn.execute("DELETE FROM benchmarks WHERE id = ?1", [id]).map(|_| ()).map_err(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::scheduler::ScanKind;
    use crate::scoring::{evaluate, Thresholds};

    fn snap(ts: i64) -> Snapshot {
        Snapshot { taken_at: ts, kind: ScanKind::Quick, system: Default::default(), sample: Default::default(), results: Default::default() }
    }

    #[test]
    fn persistence_roundtrip() {
        let db = Db::open_in_memory().expect("db");
        let mut s = db.load_settings();
        assert_eq!(s.refresh_ms, 1000);
        s.refresh_ms = 250;
        s.mask_hostname = true;
        db.save_settings(&s).expect("save");
        let s2 = db.load_settings();
        assert_eq!(s2.refresh_ms, 250);
        assert!(s2.mask_hostname);

        let sn = snap(1000);
        let h = evaluate(&sn.sample, &sn.parsed(), &Thresholds::default());
        let id = db.save_scan(&sn, &h).expect("save scan");
        db.save_scan(&snap(2000), &h).expect("save scan");
        let list = db.list_scans().expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].taken_at, 2000);
        let (loaded, health) = db.load_scan(id).expect("load");
        assert_eq!(loaded.taken_at, 1000);
        assert_eq!(health.overall, h.overall);
        db.delete_scan(id).expect("delete");
        assert_eq!(db.list_scans().expect("list").len(), 1);
        assert_eq!(db.prune(1, 2000 + 2 * 86_400).expect("prune"), 1);

        let summary = serde_json::json!({"events_per_sec": 2154.0});
        db.save_bench(10, "sysbench", &summary, &serde_json::json!({})).expect("bench");
        let b = db.list_bench("sysbench").expect("list bench");
        assert_eq!(b[0].summary["events_per_sec"], 2154.0);
    }

    #[test]
    fn migrations_are_idempotent_on_disk() {
        let dir = std::env::temp_dir().join(format!("systemhealthcheck-test-{}", std::process::id()));
        let path = dir.join("t.db");
        {
            let db = Db::open(&path).expect("open");
            db.set_value("k", "v").expect("set");
        }
        let db = Db::open(&path).expect("reopen");
        assert_eq!(db.get_value("k").as_deref(), Some("v"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
