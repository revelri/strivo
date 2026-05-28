//! strivo-clipper — highlight detection + clip extraction.
//!
//! The DAW-vision capability: take a long recording, mine the
//! candidate "interesting moments" automatically, and cut them as
//! Shorts/Reels-ready clip files. Pairs with:
//!   - Cuepoints (iter 4): visual scene changes as a density signal
//!   - Crunchr (iter 2): transcript topic shift + speaker excitement
//!   - Audio energy (future): RMS peaks via ffmpeg `astats`
//!
//! This iteration ships the first signal source (cuepoint density) and
//! the clip-cutting pipeline so the surface lights up immediately on
//! any recording that's had cuepoints extracted.
//!
//! Why density?
//!   In live captures, visual scene churn correlates with action:
//!   game deaths, kill montages, BRB returns, big chat reactions. A
//!   sliding-window density on the cuepoint timeline is a cheap
//!   first-order proxy for highlight density. Future iterations layer
//!   in audio + chat + Crunchr signals via the same Highlight scoring
//!   shape — no SPA changes needed.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use strivo_cuepoints::Cuepoint;

pub mod store;

/// Default sliding-window size used by `score_highlights`. 90s is the
/// "Shorts-friendly" length cap — windows wider than the produced clip
/// just wash out peaks.
pub const DEFAULT_WINDOW_SECS: f32 = 90.0;
/// How many top candidates to return by default.
pub const DEFAULT_TOP_K: usize = 12;
/// Clip-extraction safety pad. The cuepoint is the start of a scene
/// transition; we pull a couple of seconds back so the clip doesn't
/// start mid-cut.
pub const DEFAULT_PRE_PAD_SECS: f32 = 3.0;
/// Default clip duration. 30s = TikTok/Shorts sweet spot. Caller can
/// override per-clip.
pub const DEFAULT_CLIP_DURATION_SECS: f32 = 30.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Highlight {
    /// Centre of the window — the time we suggest the clip starts at
    /// (modulo pre-pad).
    pub time_sec: f32,
    /// Score in [0.0, 1.0] — relative density of cuepoints inside the
    /// window centred on `time_sec`.
    pub score: f32,
    /// Cuepoint count inside the window.
    pub density: usize,
    /// Suggested clip duration (s), bounded by [10, 90].
    pub suggested_duration: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightSet {
    pub recording_id: String,
    pub window_secs: f32,
    pub highlights: Vec<Highlight>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipRequest {
    pub recording_id: String,
    pub start_sec: f32,
    pub duration_sec: f32,
    /// Suggested filename stem; the caller decides extension. The
    /// crate adds the appropriate extension.
    pub stem: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipResult {
    pub recording_id: String,
    pub clip_path: String,
    pub start_sec: f32,
    pub duration_sec: f32,
    pub bytes: u64,
}

/// Slide a `window_secs`-wide window across the cuepoint timeline at
/// 5s steps; score each window by the number of cuepoints inside it;
/// return the top-K non-overlapping windows by score.
pub fn score_highlights(
    cuepoints: &[Cuepoint],
    window_secs: f32,
    top_k: usize,
) -> Vec<Highlight> {
    if cuepoints.is_empty() || window_secs <= 0.0 || top_k == 0 {
        return Vec::new();
    }
    let span_end = cuepoints
        .last()
        .map(|c| c.time_sec)
        .unwrap_or(0.0);
    let step = 5.0_f32.min(window_secs / 4.0).max(1.0);
    let half = window_secs / 2.0;

    let mut windows: Vec<Highlight> = Vec::new();
    let mut t = 0.0_f32;
    while t <= span_end + window_secs {
        let lo = (t - half).max(0.0);
        let hi = t + half;
        let density = cuepoints
            .iter()
            .filter(|c| c.time_sec >= lo && c.time_sec <= hi)
            .count();
        if density > 0 {
            windows.push(Highlight {
                time_sec: t,
                score: 0.0, // normalised below
                density,
                suggested_duration: DEFAULT_CLIP_DURATION_SECS,
            });
        }
        t += step;
    }
    if windows.is_empty() {
        return Vec::new();
    }
    let max_density = windows.iter().map(|w| w.density).max().unwrap_or(1).max(1) as f32;
    for w in &mut windows {
        w.score = (w.density as f32) / max_density;
    }
    windows.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // Greedy non-maximum suppression: take the highest-scoring window,
    // then drop every other window whose centre is within `window_secs`
    // (so we don't return overlapping clips).
    let mut picked: Vec<Highlight> = Vec::new();
    for cand in windows {
        let too_close = picked
            .iter()
            .any(|p| (p.time_sec - cand.time_sec).abs() < window_secs);
        if !too_close {
            picked.push(cand);
            if picked.len() >= top_k {
                break;
            }
        }
    }
    // Re-sort chronologically so the SPA shows them in timeline order
    // — score badge tells the user which is hottest.
    picked.sort_by(|a, b| a.time_sec.partial_cmp(&b.time_sec).unwrap_or(std::cmp::Ordering::Equal));
    picked
}

/// Clamp + tweak a user-supplied clip start/duration so it lands on
/// safe boundaries. Returns (start, duration).
pub fn clamp_request(start_sec: f32, duration_sec: f32, source_duration: Option<f32>) -> (f32, f32) {
    let mut s = start_sec.max(0.0);
    let mut d = duration_sec.clamp(10.0, 90.0);
    if let Some(src) = source_duration {
        if s + d > src {
            s = (src - d).max(0.0);
        }
        if s >= src {
            s = 0.0;
            d = d.min(src.max(1.0));
        }
    }
    (s, d)
}

/// Cut a clip from `input` into `output`. Uses `-c copy` so there's
/// no transcode — fast + lossless. ffmpeg picks the nearest keyframe
/// on the seek, so the actual start may be a fraction of a second
/// earlier than `start_sec`; the pre-pad parameter helps absorb that.
pub fn extract_clip(input: &Path, output: &Path, start_sec: f32, duration_sec: f32) -> Result<u64> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let start = format!("{start_sec:.3}");
    let dur = format!("{duration_sec:.3}");
    let status = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        // Pre-input seek for speed; -ss before -i jumps to a keyframe
        // before ffmpeg starts decoding, which is the fast path.
        .args(["-ss", &start])
        .arg("-i")
        .arg(input)
        .args(["-t", &dur, "-c", "copy", "-avoid_negative_ts", "make_zero"])
        .arg(output)
        .status()
        .context("spawn ffmpeg")?;
    if !status.success() {
        anyhow::bail!("ffmpeg exited {status}");
    }
    let meta = std::fs::metadata(output).context("stat output")?;
    Ok(meta.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(t: f32) -> Cuepoint {
        Cuepoint { time_sec: t, frame: None }
    }

    #[test]
    fn empty_cuepoints_yields_no_highlights() {
        let out = score_highlights(&[], 90.0, 12);
        assert!(out.is_empty());
    }

    #[test]
    fn dense_cluster_outranks_sparse_clusters() {
        // 3 cuepoints clustered at t=10s vs 1 at t=400s.
        let cuepoints = vec![
            cp(8.0), cp(10.0), cp(12.0),
            cp(400.0),
        ];
        let out = score_highlights(&cuepoints, 30.0, 5);
        assert!(!out.is_empty(), "expected at least 1 highlight");
        // Chronologically sorted; first highlight should be the dense
        // cluster, second (if present) should be the sparse one.
        assert!(out[0].time_sec < 50.0, "first highlight should sit near the dense cluster, got {:?}", out[0]);
        if out.len() > 1 {
            assert!(out[1].time_sec > 350.0);
            assert!(out[0].score > out[1].score);
        }
    }

    #[test]
    fn nms_drops_overlapping_windows() {
        // 10 cuepoints in a tight 20s span — without NMS we'd report
        // many overlapping windows; NMS keeps one.
        let cuepoints: Vec<Cuepoint> = (0..10).map(|i| cp(i as f32 * 2.0)).collect();
        let out = score_highlights(&cuepoints, 30.0, 5);
        assert_eq!(out.len(), 1, "expected 1 NMS-deduped highlight, got {out:?}");
    }

    #[test]
    fn top_k_caps_output_count() {
        // Many well-separated clusters; ensure top_k is honoured.
        let mut cuepoints: Vec<Cuepoint> = Vec::new();
        for i in 0..10 {
            let base = (i as f32) * 200.0;
            cuepoints.extend([cp(base), cp(base + 1.0), cp(base + 2.0)]);
        }
        let out = score_highlights(&cuepoints, 30.0, 4);
        assert!(out.len() <= 4, "expected <=4 highlights, got {}", out.len());
    }

    #[test]
    fn clamp_request_respects_source_duration() {
        let (s, d) = clamp_request(95.0, 30.0, Some(100.0));
        assert!((s + d) <= 100.0);
        assert_eq!(d, 30.0);
    }

    #[test]
    fn clamp_request_clamps_duration_into_window() {
        let (_, d) = clamp_request(0.0, 1000.0, Some(120.0));
        assert!((10.0..=90.0).contains(&d), "got duration {d}");
    }

    #[test]
    fn clamp_request_minimum_duration() {
        let (_, d) = clamp_request(0.0, 0.5, None);
        assert_eq!(d, 10.0);
    }

    #[test]
    fn scores_normalised_to_unit_range() {
        let cuepoints = vec![cp(5.0), cp(10.0), cp(60.0)];
        let out = score_highlights(&cuepoints, 30.0, 5);
        assert!(!out.is_empty());
        for h in &out {
            assert!((0.0..=1.0).contains(&h.score), "score out of range: {h:?}");
        }
    }
}
