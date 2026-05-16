//! Sqlite-backed persistence for recording jobs, catalog dedupe, and Crunchr queue.
//!
//! Single file at `{data_dir}/jobs.db`. All writes go through a single
//! `tokio::sync::Mutex<rusqlite::Connection>` to keep the schema migration story
//! trivial — the daemon is single-instance per machine, so contention is bounded
//! and the lock is short-lived.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::platform::{PlatformKind, VodEntry};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS jobs (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    payload      TEXT NOT NULL,
    state        TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    episode_dir  TEXT
);
CREATE INDEX IF NOT EXISTS idx_jobs_state ON jobs(state);

CREATE TABLE IF NOT EXISTS catalog (
    platform        TEXT NOT NULL,
    channel_id      TEXT NOT NULL,
    vod_id          TEXT NOT NULL,
    title           TEXT NOT NULL,
    published_at    TEXT,
    episode_dir     TEXT,
    recorded_at     TEXT,
    transcribed_at  TEXT,
    PRIMARY KEY (platform, channel_id, vod_id)
);
CREATE INDEX IF NOT EXISTS idx_catalog_recorded ON catalog(recorded_at);

CREATE TABLE IF NOT EXISTS crunchr_queue (
    job_id       TEXT PRIMARY KEY,
    episode_dir  TEXT NOT NULL,
    backend      TEXT NOT NULL,
    diarize      INTEGER NOT NULL DEFAULT 0,
    state        TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT
);
CREATE INDEX IF NOT EXISTS idx_crunchr_state ON crunchr_queue(state);
"#;

#[derive(Clone)]
pub struct PersistDb {
    inner: Arc<Mutex<Connection>>,
    #[allow(dead_code)]
    path: PathBuf,
}

impl PersistDb {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create data dir {}", parent.display())
            })?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open jobs.db at {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { inner: Arc::new(Mutex::new(conn)), path: path.to_path_buf() })
    }

    /// Returns `true` if this VOD is already recorded (present in `catalog`
    /// with a non-null `recorded_at`). Used by the catalog runner to skip work.
    pub async fn is_vod_recorded(
        &self,
        platform: PlatformKind,
        channel_id: &str,
        vod_id: &str,
    ) -> Result<bool> {
        let conn = self.inner.lock().await;
        let recorded: Option<Option<String>> = conn
            .query_row(
                "SELECT recorded_at FROM catalog WHERE platform=?1 AND channel_id=?2 AND vod_id=?3",
                params![platform.to_string(), channel_id, vod_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(matches!(recorded, Some(Some(s)) if !s.is_empty()))
    }

    /// Insert a discovered VOD (idempotent) — typically before queueing the
    /// recording job. `recorded_at` stays null until the job finishes.
    pub async fn upsert_catalog_entry(&self, vod: &VodEntry) -> Result<()> {
        let conn = self.inner.lock().await;
        conn.execute(
            "INSERT OR IGNORE INTO catalog (platform, channel_id, vod_id, title, published_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                vod.platform.to_string(),
                vod.channel_id,
                vod.id,
                vod.title,
                vod.published_at.map(|d| d.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    /// Mark a VOD as recorded with the resolved episode directory.
    pub async fn mark_vod_recorded(
        &self,
        platform: PlatformKind,
        channel_id: &str,
        vod_id: &str,
        episode_dir: &Path,
    ) -> Result<()> {
        let conn = self.inner.lock().await;
        conn.execute(
            "UPDATE catalog SET recorded_at = ?4, episode_dir = ?5
             WHERE platform=?1 AND channel_id=?2 AND vod_id=?3",
            params![
                platform.to_string(),
                channel_id,
                vod_id,
                chrono::Utc::now().to_rfc3339(),
                episode_dir.to_string_lossy(),
            ],
        )?;
        Ok(())
    }

    /// Mark a VOD as transcribed (after Crunchr finishes its pipeline).
    pub async fn mark_vod_transcribed(
        &self,
        platform: PlatformKind,
        channel_id: &str,
        vod_id: &str,
    ) -> Result<()> {
        let conn = self.inner.lock().await;
        conn.execute(
            "UPDATE catalog SET transcribed_at = ?4
             WHERE platform=?1 AND channel_id=?2 AND vod_id=?3",
            params![
                platform.to_string(),
                channel_id,
                vod_id,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Persist a job in any state. Uses `INSERT OR REPLACE` so callers don't
    /// have to track which transitions are inserts vs updates.
    pub async fn upsert_job(&self, job: &PersistedJob) -> Result<()> {
        let conn = self.inner.lock().await;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO jobs (id, kind, payload, state, created_at, updated_at, attempts, last_error, episode_dir)
             VALUES (?1, ?2, ?3, ?4, COALESCE((SELECT created_at FROM jobs WHERE id=?1), ?5), ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                kind=excluded.kind,
                payload=excluded.payload,
                state=excluded.state,
                updated_at=excluded.updated_at,
                attempts=excluded.attempts,
                last_error=excluded.last_error,
                episode_dir=excluded.episode_dir",
            params![
                job.id,
                job.kind,
                job.payload,
                job.state,
                now,
                job.attempts,
                job.last_error,
                job.episode_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
            ],
        )?;
        Ok(())
    }

    /// Load all jobs whose state is in the given list. Used by `recover()` on
    /// daemon startup to re-queue interrupted work.
    pub async fn load_jobs_in_states(&self, states: &[&str]) -> Result<Vec<PersistedJob>> {
        let conn = self.inner.lock().await;
        let placeholders = vec!["?"; states.len()].join(",");
        let sql = format!(
            "SELECT id, kind, payload, state, attempts, last_error, episode_dir FROM jobs WHERE state IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params_iter: Vec<&dyn rusqlite::ToSql> =
            states.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(&params_iter[..], |row| {
            Ok(PersistedJob {
                id: row.get(0)?,
                kind: row.get(1)?,
                payload: row.get(2)?,
                state: row.get(3)?,
                attempts: row.get(4)?,
                last_error: row.get(5)?,
                episode_dir: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Reconstruct `RecordingJob`s persisted by `persist_event`. Called once
    /// at daemon startup so the TUI sees its history (including
    /// interrupted-but-not-finished rows) even after a crash.
    pub async fn load_recording_jobs(&self) -> Result<Vec<crate::recording::job::RecordingJob>> {
        let conn = self.inner.lock().await;
        let mut stmt = conn.prepare(
            "SELECT payload, state, last_error FROM jobs
             WHERE kind = 'Recording'
             ORDER BY updated_at DESC
             LIMIT 500",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;

        let mut out = Vec::new();
        for r in rows {
            let (payload, state, err) = r?;
            let Ok(mut job) =
                serde_json::from_str::<crate::recording::job::RecordingJob>(&payload)
            else {
                continue;
            };
            // Force the state field to match the journal — `payload` was
            // serialized at job-creation time and may say 'queued'.
            if let Some(mapped) = map_journal_state(&state) {
                job.state = mapped;
            }
            if job.error.is_none() {
                job.error = err;
            }
            out.push(job);
        }
        Ok(out)
    }

    pub async fn upsert_crunchr_queue(&self, entry: &CrunchrQueueEntry) -> Result<()> {
        let conn = self.inner.lock().await;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO crunchr_queue (job_id, episode_dir, backend, diarize, state, created_at, updated_at, attempts, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, COALESCE((SELECT created_at FROM crunchr_queue WHERE job_id=?1), ?6), ?6, ?7, ?8)
             ON CONFLICT(job_id) DO UPDATE SET
                episode_dir=excluded.episode_dir,
                backend=excluded.backend,
                diarize=excluded.diarize,
                state=excluded.state,
                updated_at=excluded.updated_at,
                attempts=excluded.attempts,
                last_error=excluded.last_error",
            params![
                entry.job_id,
                entry.episode_dir.to_string_lossy(),
                entry.backend,
                entry.diarize as i64,
                entry.state,
                now,
                entry.attempts,
                entry.last_error,
            ],
        )?;
        Ok(())
    }

    pub async fn load_crunchr_queue_pending(&self) -> Result<Vec<CrunchrQueueEntry>> {
        let conn = self.inner.lock().await;
        let mut stmt = conn.prepare(
            "SELECT job_id, episode_dir, backend, diarize, state, attempts, last_error
             FROM crunchr_queue WHERE state IN ('queued', 'running', 'interrupted')",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CrunchrQueueEntry {
                job_id: row.get(0)?,
                episode_dir: PathBuf::from(row.get::<_, String>(1)?),
                backend: row.get(2)?,
                diarize: row.get::<_, i64>(3)? != 0,
                state: row.get(4)?,
                attempts: row.get(5)?,
                last_error: row.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub async fn delete_crunchr_queue_entry(&self, job_id: &str) -> Result<()> {
        let conn = self.inner.lock().await;
        conn.execute("DELETE FROM crunchr_queue WHERE job_id=?1", params![job_id])?;
        Ok(())
    }

    /// Mark any job in "running" or "queued" state as "interrupted". Called once
    /// at daemon startup so a crashed run leaves a clear audit trail and doesn't
    /// look like work is still in flight. Returns how many rows were updated.
    pub async fn recover_orphaned_running(&self) -> Result<u64> {
        let conn = self.inner.lock().await;
        let now = chrono::Utc::now().to_rfc3339();
        let rows = conn.execute(
            "UPDATE jobs SET state='interrupted', updated_at=?1
             WHERE state IN ('running', 'queued')",
            params![now],
        )?;
        // Same for crunchr queue.
        conn.execute(
            "UPDATE crunchr_queue SET state='interrupted', updated_at=?1
             WHERE state IN ('running')",
            params![now],
        )?;
        Ok(rows as u64)
    }
}

#[derive(Debug, Clone)]
pub struct PersistedJob {
    pub id: String,
    pub kind: String,
    pub payload: String,
    pub state: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub episode_dir: Option<PathBuf>,
}

fn map_journal_state(s: &str) -> Option<crate::recording::job::RecordingState> {
    use crate::recording::job::RecordingState as S;
    match s {
        "resolvingurl" | "resolving" => Some(S::ResolvingUrl),
        "recording" | "running" => Some(S::Recording),
        "stopping" => Some(S::Stopping),
        "finished" => Some(S::Finished),
        // 'interrupted' isn't a RecordingState variant — surface it as
        // Failed so the TUI shows the row in the failure styling.
        "failed" | "interrupted" => Some(S::Failed),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct CrunchrQueueEntry {
    pub job_id: String,
    pub episode_dir: PathBuf,
    pub backend: String,
    pub diarize: bool,
    pub state: String,
    pub attempts: i64,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catalog_dedupe_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = PersistDb::open(&dir.path().join("jobs.db")).unwrap();
        let vod = VodEntry {
            id: "abc".into(),
            platform: PlatformKind::YouTube,
            channel_id: "UC123".into(),
            title: "Episode 1".into(),
            published_at: Some(chrono::Utc::now()),
            duration: None,
            url: "https://example.com/abc".into(),
            thumbnail_url: None,
        };
        assert!(!db.is_vod_recorded(PlatformKind::YouTube, "UC123", "abc").await.unwrap());
        db.upsert_catalog_entry(&vod).await.unwrap();
        assert!(!db.is_vod_recorded(PlatformKind::YouTube, "UC123", "abc").await.unwrap());
        db.mark_vod_recorded(PlatformKind::YouTube, "UC123", "abc", std::path::Path::new("/tmp/ep")).await.unwrap();
        assert!(db.is_vod_recorded(PlatformKind::YouTube, "UC123", "abc").await.unwrap());
    }

    #[tokio::test]
    async fn job_upsert_overwrites_state() {
        let dir = tempfile::tempdir().unwrap();
        let db = PersistDb::open(&dir.path().join("jobs.db")).unwrap();
        let job = PersistedJob {
            id: "j1".into(),
            kind: "Recording".into(),
            payload: "{}".into(),
            state: "queued".into(),
            attempts: 0,
            last_error: None,
            episode_dir: None,
        };
        db.upsert_job(&job).await.unwrap();
        let mut updated = job.clone();
        updated.state = "running".into();
        updated.attempts = 1;
        db.upsert_job(&updated).await.unwrap();
        let loaded = db.load_jobs_in_states(&["running"]).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].attempts, 1);
    }
}
