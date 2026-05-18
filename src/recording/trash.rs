//! Delete-to-trash: relocate recording files into
//! `{data_dir}/trash/{YYYY-MM-DD}/` instead of unlinking them. A 7-day
//! TTL cleanup is on the roadmap but not yet implemented; users can
//! delete the trash directory manually.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::AppConfig;

fn trash_root() -> PathBuf {
    AppConfig::data_dir().join("trash")
}

/// Move `src` into a date-stamped trash directory and return the new path.
/// Collisions are disambiguated with `_N` (1..=999) followed by a uuid
/// suffix, mirroring `recording::build_output_path`.
pub fn move_to_trash(src: &Path) -> Result<PathBuf> {
    if !src.exists() {
        anyhow::bail!("source does not exist: {}", src.display());
    }
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let bucket = trash_root().join(today);
    std::fs::create_dir_all(&bucket)
        .with_context(|| format!("create trash dir {}", bucket.display()))?;

    let file_name = src
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("no file name in {}", src.display()))?;
    let initial = bucket.join(file_name);
    let dest = disambiguate(&initial);

    std::fs::rename(src, &dest)
        .with_context(|| format!("move {} -> {}", src.display(), dest.display()))?;
    Ok(dest)
}

fn disambiguate(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("trashed");
    let ext = target.extension().and_then(|s| s.to_str()).unwrap_or("");
    for n in 1..=999u32 {
        let candidate = if ext.is_empty() {
            parent.join(format!("{stem}_{n}"))
        } else {
            parent.join(format!("{stem}_{n}.{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    let uuid = uuid::Uuid::new_v4();
    if ext.is_empty() {
        parent.join(format!("{stem}_{uuid}"))
    } else {
        parent.join(format!("{stem}_{uuid}.{ext}"))
    }
}
