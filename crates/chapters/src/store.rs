//! Lightweight cache for generated chapter sets. Lives at
//! `<data_dir>/plugins/chapters/chapters.db` so it stays alongside the
//! other plugin DBs.
//!
//! Schema is one row per (recording_id, generated_at). The generator
//! is deterministic for fixed (transcript, threshold) inputs, so we
//! key on a content hash and skip the re-run when nothing changed.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::Chapter;

pub struct ChaptersStore {
    conn: Connection,
}

impl ChaptersStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chapters (
                recording_id TEXT NOT NULL,
                generated_at TEXT NOT NULL,
                source_hash  TEXT NOT NULL,
                chapters     TEXT NOT NULL,
                PRIMARY KEY (recording_id, generated_at)
             );
             CREATE INDEX IF NOT EXISTS chapters_recording_idx
                ON chapters(recording_id);",
        )?;
        Ok(Self { conn })
    }

    pub fn save(&self, recording_id: &str, source_hash: &str, chapters: &[Chapter]) -> Result<()> {
        let json = serde_json::to_string(chapters)?;
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR REPLACE INTO chapters (recording_id, generated_at, source_hash, chapters)
             VALUES (?1, ?2, ?3, ?4)",
            params![recording_id, now, source_hash, json],
        )?;
        Ok(())
    }

    pub fn latest(&self, recording_id: &str) -> Result<Option<Vec<Chapter>>> {
        let mut stmt = self.conn.prepare(
            "SELECT chapters FROM chapters
             WHERE recording_id = ?1
             ORDER BY generated_at DESC LIMIT 1",
        )?;
        let row = stmt
            .query_row([recording_id], |r| r.get::<_, String>(0))
            .optional()?;
        Ok(row
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .context("parse cached chapters")?)
    }
}

use rusqlite::OptionalExtension;
