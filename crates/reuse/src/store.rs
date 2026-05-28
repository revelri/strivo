//! SQLite store for publish drafts.
//!
//! One row per (recording_id, format). Drafts are versioned by
//! generated_at — re-running the drafter for the same recording
//! overwrites the existing rows in place.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{Format, PublishDraft};

pub struct ReuseStore {
    conn: Connection,
}

impl ReuseStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS drafts (
                recording_id TEXT NOT NULL,
                format       TEXT NOT NULL,
                draft        TEXT NOT NULL,
                generated_at TEXT NOT NULL,
                PRIMARY KEY (recording_id, format)
             );
             CREATE INDEX IF NOT EXISTS drafts_recording_idx
                ON drafts(recording_id);",
        )?;
        Ok(Self { conn })
    }

    pub fn save_set(&self, recording_id: &str, drafts: &[PublishDraft]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let tx = self.conn.unchecked_transaction()?;
        for d in drafts {
            let json = serde_json::to_string(d)?;
            tx.execute(
                "INSERT OR REPLACE INTO drafts
                    (recording_id, format, draft, generated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![recording_id, d.format.id(), json, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list(&self, recording_id: &str) -> Result<Vec<PublishDraft>> {
        let mut stmt = self.conn.prepare(
            "SELECT draft FROM drafts
             WHERE recording_id = ?1
             ORDER BY format",
        )?;
        let rows = stmt
            .query_map([recording_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|s| serde_json::from_str(&s).ok())
            .collect())
    }

    pub fn clear(&self, recording_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM drafts WHERE recording_id = ?1", [recording_id])?;
        Ok(())
    }

    pub fn delete_one(&self, recording_id: &str, format: Format) -> Result<()> {
        self.conn.execute(
            "DELETE FROM drafts WHERE recording_id = ?1 AND format = ?2",
            params![recording_id, format.id()],
        )?;
        Ok(())
    }

    pub fn latest_generated_at(&self, recording_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT MAX(generated_at) FROM drafts WHERE recording_id = ?1",
        )?;
        let row = stmt
            .query_row([recording_id], |r| r.get::<_, Option<String>>(0))
            .optional()?;
        Ok(row.flatten())
    }
}
