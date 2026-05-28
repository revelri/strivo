//! SQLite cache for generated thumbnail candidate sets.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::ThumbnailCandidate;

pub struct ThumbnailsStore {
    conn: Connection,
}

impl ThumbnailsStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS thumbnails (
                recording_id TEXT NOT NULL,
                stem         TEXT NOT NULL,
                generated_at TEXT NOT NULL,
                candidates   TEXT NOT NULL,
                PRIMARY KEY (recording_id, stem)
             );
             CREATE INDEX IF NOT EXISTS thumbnails_recording_idx
                ON thumbnails(recording_id);",
        )?;
        Ok(Self { conn })
    }

    pub fn save(&self, recording_id: &str, stem: &str, candidates: &[ThumbnailCandidate]) -> Result<()> {
        let json = serde_json::to_string(candidates)?;
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR REPLACE INTO thumbnails
                (recording_id, stem, generated_at, candidates)
             VALUES (?1, ?2, ?3, ?4)",
            params![recording_id, stem, now, json],
        )?;
        Ok(())
    }

    pub fn load(&self, recording_id: &str, stem: &str) -> Result<Option<Vec<ThumbnailCandidate>>> {
        let mut stmt = self.conn.prepare(
            "SELECT candidates FROM thumbnails
             WHERE recording_id = ?1 AND stem = ?2",
        )?;
        let row = stmt
            .query_row(params![recording_id, stem], |r| r.get::<_, String>(0))
            .optional()?;
        Ok(row
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .context("parse cached thumbnails")?)
    }
}
