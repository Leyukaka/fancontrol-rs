//! SQLite metrics store + CSV export (background writer).

use crate::MetricSink;
use fancontrol_core::{MetricSample, SensorKind, config::config_dir};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS samples (
  ts_ms INTEGER NOT NULL,
  sensor_id TEXT NOT NULL,
  value REAL NOT NULL,
  kind TEXT NOT NULL,
  unit TEXT,
  PRIMARY KEY (ts_ms, sensor_id)
);
CREATE INDEX IF NOT EXISTS idx_samples_id_ts ON samples(sensor_id, ts_ms);
"#;

/// Default DB path under the app config directory.
pub fn default_metrics_db_path() -> Option<PathBuf> {
    config_dir().ok().map(|d| d.join("metrics.sqlite"))
}

#[derive(Debug, Clone)]
pub struct SqliteStoreConfig {
    pub path: PathBuf,
    /// Purge samples older than this many days (min 1).
    pub retention_days: u32,
    /// How often the writer thread flushes pending inserts (ms).
    pub flush_ms: u64,
}

impl Default for SqliteStoreConfig {
    fn default() -> Self {
        Self {
            path: default_metrics_db_path().unwrap_or_else(|| PathBuf::from("metrics.sqlite")),
            retention_days: 7,
            flush_ms: 500,
        }
    }
}

enum StoreCmd {
    Batch(Vec<MetricSample>),
    ExportCsv {
        path: PathBuf,
        reply: SyncSender<Result<usize, String>>,
    },
    PurgeNow,
    Shutdown,
}

/// Background SQLite metrics store. `record` is non-blocking (bounded channel).
pub struct SqliteMetricsStore {
    tx: SyncSender<StoreCmd>,
}

impl SqliteMetricsStore {
    /// Spawn writer thread. Returns `None` if the DB cannot be opened.
    pub fn spawn(cfg: SqliteStoreConfig) -> Option<Self> {
        if let Some(parent) = cfg.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Probe open on the calling thread so we can report failure early.
        {
            let conn = Connection::open(&cfg.path).ok()?;
            conn.execute_batch(SCHEMA).ok()?;
        }

        let (tx, rx) = mpsc::sync_channel::<StoreCmd>(64);
        let path = cfg.path.clone();
        let retention_days = cfg.retention_days.max(1);
        let flush_ms = cfg.flush_ms.max(50);

        thread::Builder::new()
            .name("metrics-sqlite".into())
            .spawn(move || writer_loop(path, retention_days, flush_ms, rx))
            .ok()?;

        Some(Self { tx })
    }

    pub fn request_export_csv(&self, path: impl Into<PathBuf>) -> Result<usize, String> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx
            .send(StoreCmd::ExportCsv {
                path: path.into(),
                reply: reply_tx,
            })
            .map_err(|_| "metrics store worker stopped".to_string())?;
        reply_rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| "metrics CSV export timed out".to_string())?
    }

    pub fn request_purge(&self) {
        let _ = self.tx.try_send(StoreCmd::PurgeNow);
    }
}

impl MetricSink for SqliteMetricsStore {
    fn record(&mut self, batch: &[MetricSample]) {
        if batch.is_empty() {
            return;
        }
        // Drop if channel full — never block poll/UI.
        if self.tx.try_send(StoreCmd::Batch(batch.to_vec())).is_err() {
            tracing::debug!("metrics sqlite channel full; dropping batch");
        }
    }
}

impl Drop for SqliteMetricsStore {
    fn drop(&mut self) {
        let _ = self.tx.try_send(StoreCmd::Shutdown);
    }
}

fn writer_loop(path: PathBuf, retention_days: u32, flush_ms: u64, rx: Receiver<StoreCmd>) {
    let Ok(conn) = Connection::open(&path) else {
        tracing::error!(?path, "metrics sqlite open failed in worker");
        return;
    };
    if let Err(e) = conn.execute_batch(SCHEMA) {
        tracing::error!(error = %e, "metrics schema failed");
        return;
    }
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");

    let mut pending: Vec<MetricSample> = Vec::new();
    let mut last_purge = SystemTime::now();
    let purge_every = Duration::from_secs(3600);

    loop {
        // Accumulate with timeout so we batch inserts.
        match rx.recv_timeout(Duration::from_millis(flush_ms)) {
            Ok(StoreCmd::Batch(mut b)) => pending.append(&mut b),
            Ok(StoreCmd::ExportCsv { path, reply }) => {
                if !pending.is_empty() {
                    let _ = flush_inserts(&conn, &pending);
                    pending.clear();
                }
                let r = export_csv(&conn, &path);
                let _ = reply.send(r);
            }
            Ok(StoreCmd::PurgeNow) => {
                if !pending.is_empty() {
                    let _ = flush_inserts(&conn, &pending);
                    pending.clear();
                }
                purge_old(&conn, retention_days);
                last_purge = SystemTime::now();
            }
            Ok(StoreCmd::Shutdown) => {
                if !pending.is_empty() {
                    let _ = flush_inserts(&conn, &pending);
                }
                purge_old(&conn, retention_days);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !pending.is_empty() {
                    let _ = flush_inserts(&conn, &pending);
                }
                break;
            }
        }

        if !pending.is_empty() {
            if let Err(e) = flush_inserts(&conn, &pending) {
                tracing::warn!(error = %e, "metrics insert failed");
            }
            pending.clear();
        }

        if last_purge.elapsed().unwrap_or_default() >= purge_every {
            purge_old(&conn, retention_days);
            last_purge = SystemTime::now();
        }
    }
}

fn flush_inserts(conn: &Connection, batch: &[MetricSample]) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO samples (ts_ms, sensor_id, value, kind, unit) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for s in batch {
            stmt.execute(params![
                s.ts_ms,
                s.sensor_id,
                s.value,
                s.kind.as_str(),
                s.unit.as_deref(),
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn purge_old(conn: &Connection, retention_days: u32) {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let cutoff = now_ms - i64::from(retention_days) * 86_400_000;
    match conn.execute("DELETE FROM samples WHERE ts_ms < ?1", params![cutoff]) {
        Ok(n) if n > 0 => tracing::info!(deleted = n, cutoff, "metrics retention purge"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "metrics purge failed"),
    }
}

/// Options for a one-shot CSV export (used by tests / future UI filters).
#[derive(Debug, Clone, Default)]
pub struct CsvExportOptions {
    pub sensor_id_prefix: Option<String>,
}

fn export_csv(conn: &Connection, path: &Path) -> Result<usize, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    use std::io::Write;
    writeln!(file, "ts_ms,sensor_id,value,kind,unit").map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT ts_ms, sensor_id, value, kind, unit FROM samples ORDER BY ts_ms, sensor_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut n = 0usize;
    for r in rows {
        let (ts, id, val, kind, unit) = r.map_err(|e| e.to_string())?;
        let unit = unit.unwrap_or_default();
        // Escape minimal CSV (ids should not contain commas; values are numeric).
        writeln!(file, "{ts},{id},{val},{kind},{unit}").map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// Parse kind string back (tests / tooling).
#[allow(dead_code)]
fn kind_from_str(s: &str) -> SensorKind {
    match s {
        "temperature" => SensorKind::Temperature,
        "fan_rpm" => SensorKind::FanRpm,
        "voltage" => SensorKind::Voltage,
        "power" => SensorKind::Power,
        "load" => SensorKind::Load,
        _ => SensorKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fancontrol_core::SensorKind;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    #[test]
    fn sqlite_insert_and_csv_export() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.sqlite");
        let mut store = SqliteMetricsStore::spawn(SqliteStoreConfig {
            path: db.clone(),
            retention_days: 7,
            flush_ms: 50,
        })
        .expect("spawn store");

        let ts = now_ms();
        store.record(&[
            MetricSample::new(
                "host.gpu0.power.draw",
                "GPU Power",
                SensorKind::Power,
                Some("W".into()),
                42.5,
                ts,
            ),
            MetricSample::new(
                "host.gpu0",
                "GPU Core",
                SensorKind::Temperature,
                Some("°C".into()),
                55.0,
                ts,
            ),
        ]);

        // Wait for flush
        thread::sleep(Duration::from_millis(200));

        let csv_path = dir.path().join("out.csv");
        let n = store.request_export_csv(&csv_path).expect("export");
        assert!(n >= 2, "expected rows, got {n}");
        let text = std::fs::read_to_string(&csv_path).unwrap();
        assert!(text.contains("host.gpu0.power.draw"));
        assert!(text.contains("ts_ms,sensor_id"));
    }
}
