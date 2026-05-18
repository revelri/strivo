//! First-frame thumbnail extraction + cache (M5.4 substrate).
//!
//! `ffmpeg -ss <T> -i <file> -vframes 1 -q:v 4 <out.jpg>` produces a
//! small JPEG suitable for the RecordingList grid view. Results cache
//! at `{cache_dir}/recording_thumbs/<sha-of-path>.jpg`; cache entries
//! are invalidated automatically when the source file's mtime changes
//! by re-running this helper unconditionally — ffmpeg is fast enough
//! at -ss seek that re-extraction stays sub-second.
//!
//! The grid-view widget that consumes these is a separate UX commit;
//! this commit lands the extraction helper + a CLI surface so the
//! cache can be pre-warmed.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::config::AppConfig;

pub fn cache_dir() -> PathBuf {
    AppConfig::cache_dir().join("recording_thumbs")
}

/// Hash a path with the standard library DefaultHasher for use as a
/// cache filename. We pick this over a full crypto hash to keep
/// strivo-core dep-light; collisions are practically irrelevant
/// because we re-extract on every call.
fn cache_path(source: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let digest = h.finish();
    cache_dir().join(format!("{digest:016x}.jpg"))
}

/// Returns the cache path for `source` if a recent extraction is
/// already on disk. Useful for consumers (RecordingList grid) that
/// want to skip the ffmpeg call when the cache is warm.
pub fn cached(source: &Path) -> Option<PathBuf> {
    let dest = cache_path(source);
    let source_meta = std::fs::metadata(source).ok()?;
    let dest_meta = std::fs::metadata(&dest).ok()?;
    let source_mtime = source_meta.modified().ok()?;
    let dest_mtime = dest_meta.modified().ok()?;
    if dest_mtime >= source_mtime {
        Some(dest)
    } else {
        None
    }
}

/// Extract a first-frame thumbnail. Seeks to `seek_secs` (default 10)
/// so a black opening frame from a screen-capture pipeline doesn't
/// produce a uniformly dark thumbnail.
pub async fn extract(source: &Path, seek_secs: f64) -> Result<PathBuf> {
    if !source.exists() {
        return Err(anyhow!("source does not exist: {}", source.display()));
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create thumb cache dir {}", dir.display()))?;
    let dest = cache_path(source);
    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{seek_secs:.3}"),
            "-i",
        ])
        .arg(source)
        .args(["-vframes", "1", "-q:v", "4", "-vf", "scale=320:-2", "-y"])
        .arg(&dest)
        .status()
        .await
        .with_context(|| "spawn ffmpeg (is it installed?)")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg exited with status {status}"));
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_stable() {
        // Same source → same digest → same cache path.
        let p = Path::new("/tmp/recordings/abc.mkv");
        assert_eq!(cache_path(p), cache_path(p));
    }

    #[test]
    fn cache_path_distinct() {
        let a = cache_path(Path::new("/tmp/a.mkv"));
        let b = cache_path(Path::new("/tmp/b.mkv"));
        assert_ne!(a, b);
    }

    #[test]
    fn cached_missing_returns_none() {
        let p = Path::new("/nonexistent/path-that-should-not-resolve.mkv");
        assert!(cached(p).is_none());
    }
}
