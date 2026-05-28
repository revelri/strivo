//! strivo-cuepoints — scene-change cuepoint extractor.
//!
//! Wraps ffmpeg's `select=gt(scene,T),showinfo` filter chain and
//! parses the resulting `pts_time:` lines into a list of cuepoints
//! suitable for:
//!   - Editor: edit-point markers in the EDL view
//!   - Clipper: candidate clip boundaries
//!   - Chapters: visual fallback when transcript topic shift is weak
//!   - Recording info modal: a horizontal timeline of "interesting"
//!     visual moments the user can click to seek
//!
//! Pure parser + ffmpeg invocation; the cache lives in `store.rs`.
//! Slow op (full video pass) — callers should run async and surface
//! a progress hint.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub mod store;

/// Default scene-change threshold. Range is [0.0, 1.0] where 1.0 is
/// "frame is unrelated to the previous". 0.4 catches game scene swaps
/// and BRB transitions without flagging every camera pan.
pub const DEFAULT_THRESHOLD: f32 = 0.4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cuepoint {
    /// Seconds from the start of the file.
    pub time_sec: f32,
    /// Frame number, when ffmpeg reported it. Optional — older
    /// ffmpeg builds may omit it from `showinfo` output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuepointSet {
    pub recording_id: String,
    pub threshold: f32,
    pub points: Vec<Cuepoint>,
}

/// Run ffmpeg with the scene-detect filter on `input` and return the
/// detected cuepoints. Synchronous; can run for minutes on a long
/// recording.
pub fn extract_cuepoints(input: &Path, threshold: f32) -> Result<Vec<Cuepoint>> {
    let filter = format!("select='gt(scene,{threshold})',showinfo");
    let child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "info"])
        .arg("-i")
        .arg(input)
        .args(["-vf", &filter, "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn ffmpeg")?;
    let output = child
        .wait_with_output()
        .context("wait on ffmpeg")?;
    if !output.status.success() {
        anyhow::bail!(
            "ffmpeg exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(parse_showinfo(&stderr))
}

/// Parse `showinfo` lines from ffmpeg stderr. Pure function so it can
/// be unit tested without invoking ffmpeg.
///
/// Example line (split for readability):
///   `[Parsed_showinfo_1 @ 0x55..] n: 12 pts: 6144 pts_time:0.256
///    pos: 13312 fmt:yuv420p sar:1/1 …`
///
/// Newer ffmpeg builds emit `pts_time:` with a colon; we accept both
/// `pts_time:0.256` and `pts_time: 0.256`. We also coalesce duplicate
/// reports for the same frame.
pub fn parse_showinfo(stderr: &str) -> Vec<Cuepoint> {
    let mut out: Vec<Cuepoint> = Vec::new();
    for line in stderr.lines() {
        if !line.contains("showinfo") || !line.contains("pts_time") {
            continue;
        }
        let time = find_after(line, "pts_time:").and_then(parse_leading_f32);
        let frame = find_after(line, " n:")
            .or_else(|| find_after(line, "n:"))
            .and_then(parse_leading_u64);
        if let Some(t) = time {
            let cp = Cuepoint { time_sec: t, frame };
            // Drop near-duplicates within 50ms — ffmpeg sometimes
            // reports the same frame twice across the encoder + filter.
            if out.last().map(|p| (p.time_sec - t).abs() > 0.05).unwrap_or(true) {
                out.push(cp);
            }
        }
    }
    out
}

fn find_after<'a>(line: &'a str, needle: &str) -> Option<&'a str> {
    let idx = line.find(needle)?;
    Some(line[idx + needle.len()..].trim_start())
}

fn parse_leading_f32(s: &str) -> Option<f32> {
    let len = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .count();
    s.get(..len).and_then(|t| t.parse().ok())
}

fn parse_leading_u64(s: &str) -> Option<u64> {
    let len = s.chars().take_while(|c| c.is_ascii_digit()).count();
    s.get(..len).and_then(|t| t.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_showinfo_line() {
        let stderr =
            "[Parsed_showinfo_1 @ 0x55] n: 12 pts: 6144 pts_time:0.256 pos: 13312 fmt:yuv420p sar:1/1\n";
        let out = parse_showinfo(stderr);
        assert_eq!(out.len(), 1);
        assert!((out[0].time_sec - 0.256).abs() < 1e-4);
        assert_eq!(out[0].frame, Some(12));
    }

    #[test]
    fn parse_handles_space_after_colon() {
        let stderr =
            "[Parsed_showinfo_1 @ 0x55] n: 99 pts_time: 5.5 pos: 1 fmt:yuv420p\n";
        let out = parse_showinfo(stderr);
        assert_eq!(out.len(), 1);
        assert!((out[0].time_sec - 5.5).abs() < 1e-4);
    }

    #[test]
    fn parse_dedupes_near_duplicate_timestamps() {
        let stderr = "\
[Parsed_showinfo_1 @ 0x55] n: 1 pts_time:1.000 fmt:yuv420p
[Parsed_showinfo_1 @ 0x55] n: 1 pts_time:1.001 fmt:yuv420p
[Parsed_showinfo_1 @ 0x55] n: 2 pts_time:2.500 fmt:yuv420p
";
        let out = parse_showinfo(stderr);
        assert_eq!(out.len(), 2, "got {out:?}");
        assert!((out[0].time_sec - 1.0).abs() < 1e-4);
        assert!((out[1].time_sec - 2.5).abs() < 1e-4);
    }

    #[test]
    fn parse_ignores_non_showinfo_lines() {
        let stderr =
            "frame=  100 fps= 25 q=-1.0 size=   N/A time=00:00:04.00 bitrate=N/A speed= 0.5x\n";
        assert!(parse_showinfo(stderr).is_empty());
    }

    #[test]
    fn parse_keeps_long_offsets() {
        let stderr = "[Parsed_showinfo_1 @ 0x55] n: 7200 pts_time:3601.234 fmt:yuv420p\n";
        let out = parse_showinfo(stderr);
        assert_eq!(out.len(), 1);
        assert!((out[0].time_sec - 3601.234).abs() < 1e-3);
        assert_eq!(out[0].frame, Some(7200));
    }
}
