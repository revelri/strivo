//! Command-log recorder + preset save/load. (X3.)
//!
//! Companion to the palette dispatcher — every successfully-dispatched
//! [`crate::tui::keymap::KeyAction`] gets appended to
//! [`crate::app::AppState::command_log`] (capped at 256 entries). This
//! module turns that ring into a saved preset JSON the user can replay
//! later via `palette:run-preset <name>`.
//!
//! The preset format is intentionally simple — a name + a list of
//! action names — so it's hand-editable. Eventual EDL integration
//! lands when keymap actions get a stable mapping onto `EdlOp`s; for
//! now the recorder is a sidecar.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Where preset logs live. Honors `STRIVO_DATA_DIR` for tests; falls
/// back to `~/.local/share/strivo/command-logs/`.
pub fn dir() -> PathBuf {
    if let Ok(custom) = std::env::var("STRIVO_DATA_DIR") {
        return PathBuf::from(custom).join("command-logs");
    }
    directories::BaseDirs::new()
        .map(|d| d.data_local_dir().join("strivo").join("command-logs"))
        .unwrap_or_else(|| PathBuf::from(".").join("strivo").join("command-logs"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPreset {
    pub name: String,
    /// Ordered list of [`crate::tui::keymap::KeyAction`] names (from
    /// `KeyAction::name()`). Replay iterates these.
    pub actions: Vec<String>,
    pub saved_at: String,
}

impl CommandPreset {
    pub fn new(name: impl Into<String>, actions: Vec<String>) -> Self {
        Self {
            name: name.into(),
            actions,
            saved_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Snapshot the current command log under `<name>.json`. Atomic write
/// (`.tmp` then rename). Returns the final path.
pub fn save(name: &str, log: &[String]) -> Result<PathBuf> {
    if name.is_empty() {
        anyhow::bail!("preset name cannot be empty");
    }
    let trimmed: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(*c, '-' | '_'))
        .collect();
    if trimmed.is_empty() {
        anyhow::bail!("preset name must contain alphanumeric/-/_ chars");
    }

    let dir = dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("mkdir {}", dir.display()))?;
    let path = dir.join(format!("{trimmed}.json"));
    let preset = CommandPreset::new(trimmed.clone(), log.to_vec());
    let text = serde_json::to_string_pretty(&preset)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} → {}", tmp.display(), path.display()))?;
    Ok(path)
}

pub fn load(path: &Path) -> Result<CommandPreset> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let p: CommandPreset = serde_json::from_str(&text)
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(p)
}

/// List every preset on disk. Quiet about parse failures so one bad
/// file doesn't shadow the rest.
pub fn list() -> Vec<CommandPreset> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Ok(preset) = load(&p) {
            out.push(preset);
        }
    }
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("STRIVO_DATA_DIR", tmp.path());
        let actions = vec!["Quit".to_string(), "EventLogToggle".to_string()];
        let path = save("daily-cleanup", &actions).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.name, "daily-cleanup");
        assert_eq!(loaded.actions, actions);
    }

    #[test]
    fn empty_name_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("STRIVO_DATA_DIR", tmp.path());
        assert!(save("", &[]).is_err());
        // Names with only invalid chars also reject.
        assert!(save("!!! ", &[]).is_err());
    }

    #[test]
    fn name_sanitization() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("STRIVO_DATA_DIR", tmp.path());
        let path = save("good name!", &[]).unwrap();
        // The space + `!` get stripped; result file is `goodname.json`.
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("goodname"));
    }
}
