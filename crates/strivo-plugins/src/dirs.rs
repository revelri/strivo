//! Shared directory helpers for first-party plugins.

use std::path::PathBuf;

/// Where M5.1 clip-export drops finished clips. Falls back to a
/// relative `./clips` if the OS-conventional data dir can't be resolved.
pub fn clips_dir() -> PathBuf {
    strivo_core::config::AppConfig::data_dir().join("clips")
}
