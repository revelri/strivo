//! SQLite cache for cuepoint sets.
//!
//! Cuepoint extraction is slow (full video pass). The store keys on
//! (recording_id, threshold) so re-asking for the same parameters is
//! a no-op.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{Cuepoint, CuepointSet};

pub struct CuepointsStore {
    conn: Connection,
}

impl CuepointsStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cuepoints (
                recording_id TEXT NOT NULL,
                threshold    REAL NOT NULL,
                generated_at TEXT NOT NULL,
                points       TEXT NOT NULL,
                PRIMARY KEY (recording_id, threshold)
             );
             CREATE INDEX IF NOT EXISTS cuepoints_recording_idx
                ON cuepoints(recording_id);",
        )?;
        Ok(Self { conn })
    }

    pub fn save(&self, set: &CuepointSet) -> Result<()> {
        let json = serde_json::to_string(&set.points)?;
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR REPLACE INTO cuepoints
                (recording_id, threshold, generated_at, points)
             VALUES (?1, ?2, ?3, ?4)",
            params![set.recording_id, set.threshold as f64, now, json],
        )?;
        Ok(())
    }

    pub fn load(&self, recording_id: &str, threshold: f32) -> Result<Option<Vec<Cuepoint>>> {
        let mut stmt = self.conn.prepare(
            "SELECT points FROM cuepoints
             WHERE recording_id = ?1 AND threshold = ?2",
        )?;
        let row = stmt
            .query_row(params![recording_id, threshold as f64], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        Ok(row
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .context("parse cached cuepoints")?)
    }
}
