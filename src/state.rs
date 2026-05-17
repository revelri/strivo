//! TUI-managed runtime state — distinct from `config.toml` (user-authored
//! values). Persisted at `{state_dir}/state.json`.
//!
//! Decision from M2.1.a (docs/SETTINGS-COVERAGE.md):
//! - `config.toml` holds user-authored values (recording_dir, schedules,
//!   auto-record entries, theme overrides, …).
//! - `state.json` holds TUI-managed values (watched flags, last-used
//!   selection, sidebar state, …) — things the user doesn't author by
//!   hand but that ought to survive a restart.
//!
//! Migration: the legacy `watched.json` (flat `[Uuid]`) is folded into
//! the new structure on first load if `state.json` doesn't yet exist.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::AppConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TuiState {
    /// Recording UUIDs the user has played.
    #[serde(default)]
    pub watched: HashSet<Uuid>,

    /// Last RecordingList row by Uuid. Restored to AppState on startup
    /// so resuming the TUI lands you on the row you left.
    #[serde(default)]
    pub selected_recording: Option<Uuid>,

    /// Active pane the user last had focused. Restored on startup so
    /// the cursor lands where they left it.
    #[serde(default)]
    pub last_pane: Option<String>,

    /// Channel marks (M4.2.b — yazi audit §11). Map a single lowercase
    /// char to a channel id; `'{c}` jumps to it. Insertion-ordered via
    /// BTreeMap so serialization is deterministic.
    #[serde(default)]
    pub marks: BTreeMap<char, String>,
}

fn path() -> PathBuf {
    AppConfig::state_dir().join("state.json")
}

fn legacy_watched_path() -> PathBuf {
    AppConfig::state_dir().join("watched.json")
}

impl TuiState {
    pub fn load() -> Self {
        let p = path();
        if let Ok(contents) = std::fs::read_to_string(&p) {
            match serde_json::from_str::<Self>(&contents) {
                Ok(s) => return s,
                Err(e) => tracing::warn!("state.json parse failed: {e} — using defaults"),
            }
        }
        // Legacy path — migrate watched.json if it's there.
        if let Ok(contents) = std::fs::read_to_string(legacy_watched_path()) {
            if let Ok(list) = serde_json::from_str::<Vec<Uuid>>(&contents) {
                tracing::info!("migrating legacy watched.json into state.json");
                let migrated = TuiState {
                    watched: list.into_iter().collect(),
                    ..Default::default()
                };
                migrated.save();
                let _ = std::fs::remove_file(legacy_watched_path());
                return migrated;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&p, s) {
                    tracing::warn!("state.json write failed: {e}");
                }
            }
            Err(e) => tracing::warn!("state.json serialize failed: {e}"),
        }
    }
}
