//! SQLite cache for highlight sets + extracted clip records.
//!
//! Two tables: one for highlight candidates (per recording, per
//! window), one for the actual cut clip files. The store survives
//! restarts so the SPA can show the clip status without re-running
//! the detection.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{ClipResult, Highlight};

pub struct ClipperStore {
    conn: Connection,
}

impl ClipperStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS highlights (
                recording_id TEXT NOT NULL,
                window_secs  REAL NOT NULL,
                generated_at TEXT NOT NULL,
                highlights   TEXT NOT NULL,
                PRIMARY KEY (recording_id, window_secs)
             );
             CREATE TABLE IF NOT EXISTS clips (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                recording_id  TEXT NOT NULL,
                clip_path     TEXT NOT NULL,
                start_sec     REAL NOT NULL,
                duration_sec  REAL NOT NULL,
                bytes         INTEGER NOT NULL,
                cut_at        TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS clips_recording_idx ON clips(recording_id);",
        )?;
        Ok(Self { conn })
    }

    pub fn save_highlights(
        &self,
        recording_id: &str,
        window_secs: f32,
        highlights: &[Highlight],
    ) -> Result<()> {
        let json = serde_json::to_string(highlights)?;
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR REPLACE INTO highlights
                (recording_id, window_secs, generated_at, highlights)
             VALUES (?1, ?2, ?3, ?4)",
            params![recording_id, window_secs as f64, now, json],
        )?;
        Ok(())
    }

    pub fn load_highlights(
        &self,
        recording_id: &str,
        window_secs: f32,
    ) -> Result<Option<Vec<Highlight>>> {
        let mut stmt = self.conn.prepare(
            "SELECT highlights FROM highlights
             WHERE recording_id = ?1 AND window_secs = ?2",
        )?;
        let row = stmt
            .query_row(params![recording_id, window_secs as f64], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        Ok(row
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .context("parse cached highlights")?)
    }

    pub fn save_clip(&self, clip: &ClipResult) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO clips
                (recording_id, clip_path, start_sec, duration_sec, bytes, cut_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                clip.recording_id,
                clip.clip_path,
                clip.start_sec as f64,
                clip.duration_sec as f64,
                clip.bytes as i64,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn list_clips(&self, recording_id: &str) -> Result<Vec<ClipResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT recording_id, clip_path, start_sec, duration_sec, bytes
             FROM clips WHERE recording_id = ?1 ORDER BY start_sec",
        )?;
        let rows = stmt
            .query_map([recording_id], |r| {
                Ok(ClipResult {
                    recording_id: r.get(0)?,
                    clip_path: r.get(1)?,
                    start_sec: r.get::<_, f64>(2)? as f32,
                    duration_sec: r.get::<_, f64>(3)? as f32,
                    bytes: r.get::<_, i64>(4)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
