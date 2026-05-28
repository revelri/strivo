//! SQLite store for EDL drafts + revision history.
//!
//! Two tables:
//! - `edls` — one row per recording, the latest EDL.
//! - `revisions` — append-only history. Every [`EdlStore::save`] inserts a
//!   row keyed by `(recording_id, revision_id)` with a label describing the
//!   edit ("manual edit", "trim dead air", "revert to v3", …). The SPA
//!   surfaces the list so the user can revert across saves — DAW-style undo
//!   that survives a page reload.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::Edl;

pub struct EdlStore {
    conn: Connection,
}

/// Lightweight metadata row for the revisions list.
#[derive(Debug, Clone, Serialize)]
pub struct RevisionMeta {
    pub revision_id: i64,
    pub recording_id: String,
    pub label: String,
    pub created_at: String,
    pub cut_count: usize,
    pub total_duration_sec: f32,
}

impl EdlStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS edls (
                recording_id TEXT PRIMARY KEY,
                edl_json     TEXT NOT NULL,
                updated_at   TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS revisions (
                revision_id  INTEGER PRIMARY KEY AUTOINCREMENT,
                recording_id TEXT NOT NULL,
                edl_json     TEXT NOT NULL,
                label        TEXT NOT NULL,
                created_at   TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_revisions_recording ON revisions(recording_id, created_at);",
        )?;
        Ok(Self { conn })
    }

    /// Save with default label `manual edit`. Kept for callers that don't
    /// know the edit kind.
    pub fn save(&self, edl: &Edl) -> Result<()> {
        self.save_with_label(edl, "manual edit")
    }

    /// Save and append a revision tagged with `label`.
    pub fn save_with_label(&self, edl: &Edl, label: &str) -> Result<()> {
        let json = serde_json::to_string(edl)?;
        let now = chrono::Utc::now().to_rfc3339();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO edls (recording_id, edl_json, updated_at)
             VALUES (?1, ?2, ?3)",
            params![edl.recording_id, json, now],
        )?;
        tx.execute(
            "INSERT INTO revisions (recording_id, edl_json, label, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![edl.recording_id, json, label, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load(&self, recording_id: &str) -> Result<Option<Edl>> {
        let mut stmt = self
            .conn
            .prepare("SELECT edl_json FROM edls WHERE recording_id = ?1")?;
        let row = stmt
            .query_row([recording_id], |r| r.get::<_, String>(0))
            .optional()?;
        Ok(row
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .context("parse cached EDL")?)
    }

    pub fn clear(&self, recording_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM edls WHERE recording_id = ?1", [recording_id])?;
        Ok(())
    }

    /// Newest-first list of revision metadata for `recording_id`. Cheap —
    /// reads `cut_count` + `total_duration_sec` from the stored JSON's cut
    /// array length and end-minus-start sum.
    pub fn list_revisions(&self, recording_id: &str, limit: usize) -> Result<Vec<RevisionMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT revision_id, edl_json, label, created_at
             FROM revisions
             WHERE recording_id = ?1
             ORDER BY revision_id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![recording_id, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (revision_id, json, label, created_at) = row?;
            let edl: Edl = serde_json::from_str(&json).context("parse revision JSON")?;
            let cut_count = edl.cuts.len();
            let total_duration_sec = edl.total_duration();
            out.push(RevisionMeta {
                revision_id,
                recording_id: recording_id.to_string(),
                label,
                created_at,
                cut_count,
                total_duration_sec,
            });
        }
        Ok(out)
    }

    /// Load a specific revision's EDL by id. Returns None when the revision
    /// doesn't exist or belongs to a different recording.
    pub fn load_revision(&self, recording_id: &str, revision_id: i64) -> Result<Option<Edl>> {
        let mut stmt = self.conn.prepare(
            "SELECT edl_json FROM revisions
             WHERE recording_id = ?1 AND revision_id = ?2",
        )?;
        let row = stmt
            .query_row(params![recording_id, revision_id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        Ok(row.map(|j| serde_json::from_str(&j)).transpose()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cut, CutKind};

    fn temp_store() -> (EdlStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = EdlStore::open(&dir.path().join("editor.db")).unwrap();
        (store, dir)
    }

    fn sample_edl(rec: &str, n: usize) -> Edl {
        Edl {
            recording_id: rec.into(),
            cuts: (0..n)
                .map(|i| Cut {
                    kind: CutKind::Source { source_path: "/tmp/x.mkv".into() },
                    start_sec: i as f32 * 10.0,
                    end_sec: i as f32 * 10.0 + 5.0,
                    fade_in_sec: 0.0,
                    fade_out_sec: 0.0,
                })
                .collect(),
        }
    }

    #[test]
    fn save_writes_revision_too() {
        let (store, _d) = temp_store();
        store.save(&sample_edl("rec1", 2)).unwrap();
        let revs = store.list_revisions("rec1", 50).unwrap();
        assert_eq!(revs.len(), 1);
        assert_eq!(revs[0].cut_count, 2);
        assert_eq!(revs[0].label, "manual edit");
    }

    #[test]
    fn multiple_saves_accumulate_revisions_newest_first() {
        let (store, _d) = temp_store();
        store.save_with_label(&sample_edl("r", 1), "a").unwrap();
        store.save_with_label(&sample_edl("r", 2), "b").unwrap();
        store.save_with_label(&sample_edl("r", 3), "c").unwrap();
        let revs = store.list_revisions("r", 50).unwrap();
        assert_eq!(revs.len(), 3);
        assert_eq!(revs[0].label, "c");
        assert_eq!(revs[1].label, "b");
        assert_eq!(revs[2].label, "a");
        assert!(revs[0].revision_id > revs[1].revision_id);
    }

    #[test]
    fn limit_clamps_returned_rows() {
        let (store, _d) = temp_store();
        for i in 0..7 {
            store.save_with_label(&sample_edl("r", i + 1), &format!("v{i}")).unwrap();
        }
        let revs = store.list_revisions("r", 3).unwrap();
        assert_eq!(revs.len(), 3);
    }

    #[test]
    fn revisions_are_scoped_to_recording_id() {
        let (store, _d) = temp_store();
        store.save_with_label(&sample_edl("alpha", 1), "x").unwrap();
        store.save_with_label(&sample_edl("beta", 2), "y").unwrap();
        let alpha = store.list_revisions("alpha", 50).unwrap();
        let beta = store.list_revisions("beta", 50).unwrap();
        assert_eq!(alpha.len(), 1);
        assert_eq!(beta.len(), 1);
        assert_eq!(alpha[0].label, "x");
        assert_eq!(beta[0].label, "y");
    }

    #[test]
    fn load_revision_returns_exact_state() {
        let (store, _d) = temp_store();
        store.save_with_label(&sample_edl("r", 5), "five").unwrap();
        store.save_with_label(&sample_edl("r", 1), "one").unwrap();
        let revs = store.list_revisions("r", 50).unwrap();
        let five = revs.iter().find(|r| r.label == "five").unwrap();
        let edl = store.load_revision("r", five.revision_id).unwrap().unwrap();
        assert_eq!(edl.cuts.len(), 5);
    }

    #[test]
    fn load_revision_rejects_mismatched_recording_id() {
        let (store, _d) = temp_store();
        store.save_with_label(&sample_edl("a", 2), "x").unwrap();
        let revs = store.list_revisions("a", 50).unwrap();
        let got = store.load_revision("b", revs[0].revision_id).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn load_revision_returns_none_for_unknown_id() {
        let (store, _d) = temp_store();
        store.save_with_label(&sample_edl("r", 1), "x").unwrap();
        let got = store.load_revision("r", 99999).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn metadata_reflects_stored_cut_count_and_duration() {
        let (store, _d) = temp_store();
        // 3 cuts × 5 seconds each = 15s total
        store.save_with_label(&sample_edl("r", 3), "z").unwrap();
        let revs = store.list_revisions("r", 50).unwrap();
        assert_eq!(revs[0].cut_count, 3);
        assert!((revs[0].total_duration_sec - 15.0).abs() < 1e-3);
    }

    #[test]
    fn clear_does_not_delete_history() {
        // Explicit design choice: revisions are append-only; clearing the
        // "current" EDL still lets the user recover from history.
        let (store, _d) = temp_store();
        store.save_with_label(&sample_edl("r", 2), "x").unwrap();
        store.clear("r").unwrap();
        assert!(store.load("r").unwrap().is_none());
        let revs = store.list_revisions("r", 50).unwrap();
        assert_eq!(revs.len(), 1);
    }

    #[test]
    fn save_then_load_matches() {
        let (store, _d) = temp_store();
        let edl = sample_edl("r", 4);
        store.save(&edl).unwrap();
        let back = store.load("r").unwrap().unwrap();
        assert_eq!(back.cuts.len(), 4);
    }
}
