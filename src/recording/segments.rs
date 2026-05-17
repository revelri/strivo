//! Stream-gap resume helpers (M5.5 substrate).
//!
//! When a long stream drops mid-recording, FFmpeg can't append to a
//! Matroska file in place. The pattern that works is: produce N
//! independent MKV segments (one per FFmpeg invocation) and run
//! `mkvmerge --append` once the user wants the joined file. This
//! module exposes:
//!
//! - `segment_path(base, n)` — derive `<base>_part2.mkv` for the Nth
//!   resume so segments sit next to the original recording
//! - `merge_segments(sources, dest)` — spawn `mkvmerge -o <dest>
//!   <s0> + <s1> + …`; the `+` operator pre-element is the append
//!   syntax mkvmerge expects
//!
//! Full automatic resume orchestration (detect FFmpeg exit while
//! channel is still live → re-resolve URL → spawn next segment) lives
//! in the daemon's recording supervisor; that change is sizeable and
//! deserves real-world soak testing before landing. This module is
//! the building block the supervisor will call into.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

/// Compute the path for an Nth resume segment (`n >= 2`). For
/// `base = "channel_2026-05-16.mkv"` and `n = 2` the result is
/// `"channel_2026-05-16_part2.mkv"`. Extension is preserved.
pub fn segment_path(base: &Path, n: u32) -> PathBuf {
    if n <= 1 {
        return base.to_path_buf();
    }
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("recording");
    let ext = base.extension().and_then(|s| s.to_str()).unwrap_or("mkv");
    parent.join(format!("{stem}_part{n}.{ext}"))
}

/// Merge `sources` into `dest` with `mkvmerge`. The first source
/// owns the timeline; subsequent sources are appended verbatim. The
/// caller is responsible for ordering `sources` chronologically.
pub fn merge_segments(sources: &[PathBuf], dest: &Path) -> Result<()> {
    if sources.is_empty() {
        return Err(anyhow!("merge_segments: empty source list"));
    }
    for s in sources {
        if !s.exists() {
            return Err(anyhow!("merge source does not exist: {}", s.display()));
        }
    }
    let mut cmd = Command::new("mkvmerge");
    cmd.arg("-o").arg(dest).arg(&sources[0]);
    for s in &sources[1..] {
        cmd.arg("+").arg(s);
    }
    let status = cmd
        .status()
        .with_context(|| "spawn mkvmerge (is mkvtoolnix installed?)")?;
    if !status.success() {
        return Err(anyhow!("mkvmerge exited with status {status}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_path_first_is_base() {
        let base = Path::new("/tmp/rec.mkv");
        assert_eq!(segment_path(base, 1), base);
        assert_eq!(segment_path(base, 0), base);
    }

    #[test]
    fn segment_path_part_n() {
        let base = Path::new("/tmp/rec.mkv");
        assert_eq!(
            segment_path(base, 2),
            PathBuf::from("/tmp/rec_part2.mkv")
        );
        assert_eq!(
            segment_path(base, 5),
            PathBuf::from("/tmp/rec_part5.mkv")
        );
    }

    #[test]
    fn merge_segments_empty_fails() {
        let dest = Path::new("/tmp/never-created.mkv");
        let err = merge_segments(&[], dest).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn merge_segments_missing_source_fails() {
        let dest = Path::new("/tmp/never-created.mkv");
        let srcs = vec![PathBuf::from("/nonexistent/source.mkv")];
        let err = merge_segments(&srcs, dest).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }
}
