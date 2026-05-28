//! strivo-thumbnails — frame extraction + saliency ranking + facecam crops.
//!
//! The DAW-vision capability: every clip and every published video
//! needs a thumbnail. Manual frame-hunting is hours per stream. This
//! plugin samples frames at user-chosen timestamps (cuepoint times,
//! highlight peaks, even-interval samples) and ranks them by visual
//! complexity so the streamer picks from a short list of "interesting"
//! candidates instead of scrubbing 8 hours of footage.
//!
//! Three primitives:
//!
//!   1. `extract_frame(input, timestamp, output)` — single-frame
//!      ffmpeg grab. Fast keyframe seek.
//!   2. `score_frame(path)` — saliency proxy. JPEG file size grows
//!      monotonically with visual entropy; combine that with a
//!      content-byte-variance term so we don't get tricked by raw PNG
//!      or padded fixtures. Deterministic + unit-testable from any
//!      byte stream.
//!   3. `pick_facecam_crop(w, h, position)` — returns a 9:16 vertical
//!      crop rectangle around the streamer's facecam corner (top-left,
//!      top-right, bottom-left, bottom-right). Pure math; tested
//!      against canonical resolutions.
//!
//! Composed pipeline `generate_candidates(...)` walks a list of source
//! timestamps, extracts frames, scores them, and returns the ranked
//! candidate set with optional facecam-cropped variants per pick.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub mod store;

/// Where on the screen the streamer's facecam usually sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacecamCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Default for FacecamCorner {
    fn default() -> Self {
        Self::TopRight
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbnailCandidate {
    pub time_sec: f32,
    pub path: String,
    /// Saliency score in [0.0, 1.0] normalised across the candidate set.
    pub score: f32,
    /// Raw score components for transparency.
    pub bytes: u64,
    pub variance: u64,
    /// Optional vertical-crop candidate path (facecam-targeted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateOptions {
    pub timestamps: Vec<f32>,
    /// Output directory; created if missing.
    pub out_dir: PathBuf,
    /// Filename stem prefix — actual files are `<stem>_<idx>.jpg`.
    pub stem: String,
    /// When set, also emit a 9:16 vertical crop per frame at the
    /// requested corner. Source resolution comes from ffprobe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facecam: Option<FacecamCorner>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    pub recording_id: String,
    pub candidates: Vec<ThumbnailCandidate>,
}

/// Extract a single frame at `time_sec`. `-ss` before `-i` makes
/// ffmpeg jump to the nearest keyframe, which is fine for thumbnails.
pub fn extract_frame(input: &Path, time_sec: f32, output: &Path) -> Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let ts = format!("{time_sec:.3}");
    let status = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(["-ss", &ts])
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1", "-q:v", "3"])
        .arg(output)
        .status()
        .context("spawn ffmpeg")?;
    if !status.success() {
        anyhow::bail!("ffmpeg exited {status}");
    }
    Ok(())
}

/// Same as `extract_frame` but with an additional crop filter that
/// emits a 9:16 vertical frame for the facecam corner.
pub fn extract_frame_cropped(
    input: &Path,
    time_sec: f32,
    output: &Path,
    crop: CropRect,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let ts = format!("{time_sec:.3}");
    let filter = format!("crop={}:{}:{}:{}", crop.w, crop.h, crop.x, crop.y);
    let status = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(["-ss", &ts])
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1", "-q:v", "3"])
        .args(["-vf", &filter])
        .arg(output)
        .status()
        .context("spawn ffmpeg")?;
    if !status.success() {
        anyhow::bail!("ffmpeg crop exited {status}");
    }
    Ok(())
}

/// Saliency score on a frame at `path`. Combines file size and byte
/// variance so it works on JPEG (compressed size correlates with
/// entropy) and on raw fixtures (variance carries the signal).
/// Returns `(bytes, variance, raw_score)` so callers can normalise.
pub fn score_frame(path: &Path) -> Result<(u64, u64, f64)> {
    let bytes = std::fs::read(path).context("read frame")?;
    Ok(score_bytes(&bytes))
}

/// Pure scoring on raw bytes — split out so the unit tests don't need
/// a filesystem fixture.
pub fn score_bytes(bytes: &[u8]) -> (u64, u64, f64) {
    let n = bytes.len() as u64;
    if bytes.is_empty() {
        return (0, 0, 0.0);
    }
    // Variance of the byte values. Uses an online formula to stay
    // within u64 across a 30 MB JPEG.
    let mut sum: u64 = 0;
    let mut sum_sq: u128 = 0;
    for &b in bytes {
        sum += b as u64;
        sum_sq += (b as u128) * (b as u128);
    }
    let mean = sum as f64 / bytes.len() as f64;
    let variance = (sum_sq as f64 / bytes.len() as f64) - mean * mean;
    let variance_u = variance.max(0.0) as u64;
    // Raw score: log(bytes+1) + variance scaled. The log keeps very
    // large files from dominating; variance separates near-uniform
    // padding from real content.
    let raw = ((n as f64 + 1.0).ln() + variance / 128.0).max(0.0);
    (n, variance_u, raw)
}

/// Vertical 9:16 crop for a facecam corner. Pure math; tested.
pub fn pick_facecam_crop(width: u32, height: u32, corner: FacecamCorner) -> CropRect {
    // 9:16 vertical. Aim for a window that's the full height (or
    // 95% of it for breathing room) and matches the aspect ratio,
    // anchored to the chosen corner so the streamer's overlay stays
    // inside the crop.
    let target_h = height.saturating_mul(95) / 100;
    let target_w_unclamped = (target_h as u64 * 9 / 16) as u32;
    // If the source isn't 16:9 the target width could exceed source.
    let w = target_w_unclamped.min(width);
    let h = target_h.min(height);
    let x = match corner {
        FacecamCorner::TopLeft | FacecamCorner::BottomLeft => 0,
        FacecamCorner::TopRight | FacecamCorner::BottomRight => width.saturating_sub(w),
    };
    let y = match corner {
        FacecamCorner::TopLeft | FacecamCorner::TopRight => 0,
        FacecamCorner::BottomLeft | FacecamCorner::BottomRight => height.saturating_sub(h),
    };
    CropRect { x, y, w, h }
}

/// Walk `opts.timestamps`, extract frames, score them, optionally
/// emit a facecam-cropped variant per frame. Returns the candidate
/// set ranked by score (descending).
pub fn generate_candidates(
    input: &Path,
    source_resolution: (u32, u32),
    opts: &GenerateOptions,
    recording_id: &str,
) -> Result<GenerateResult> {
    let mut candidates: Vec<ThumbnailCandidate> = Vec::new();
    std::fs::create_dir_all(&opts.out_dir).ok();
    for (i, t) in opts.timestamps.iter().copied().enumerate() {
        let out = opts.out_dir.join(format!("{}_{:03}.jpg", opts.stem, i));
        extract_frame(input, t, &out).with_context(|| format!("extract at {t}s"))?;
        let (bytes, variance, raw) = score_frame(&out)?;
        let crop_path = if let Some(corner) = opts.facecam {
            let crop = pick_facecam_crop(source_resolution.0, source_resolution.1, corner);
            let crop_out = opts.out_dir.join(format!("{}_{:03}_facecam.jpg", opts.stem, i));
            extract_frame_cropped(input, t, &crop_out, crop)?;
            Some(crop_out.to_string_lossy().to_string())
        } else {
            None
        };
        candidates.push(ThumbnailCandidate {
            time_sec: t,
            path: out.to_string_lossy().to_string(),
            score: raw as f32, // normalised below
            bytes,
            variance,
            crop_path,
        });
    }
    normalise_scores(&mut candidates);
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(GenerateResult { recording_id: recording_id.to_string(), candidates })
}

/// Map raw scores into [0,1] across the set so the SPA can render a
/// percent bar. Public so callers that already have raw scores (from
/// `score_frame` directly) can normalise the same way.
pub fn normalise_scores(set: &mut [ThumbnailCandidate]) {
    let max = set.iter().map(|c| c.score).fold(0.0_f32, f32::max);
    if max <= 0.0 {
        return;
    }
    for c in set {
        c.score /= max;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(time: f32, raw: f32) -> ThumbnailCandidate {
        ThumbnailCandidate {
            time_sec: time,
            path: String::new(),
            score: raw,
            bytes: 0,
            variance: 0,
            crop_path: None,
        }
    }

    #[test]
    fn crop_top_right_1080p() {
        let c = pick_facecam_crop(1920, 1080, FacecamCorner::TopRight);
        assert_eq!(c.h, 1026); // 95% of 1080
        assert_eq!(c.w, 577); // 1026 * 9 / 16 = 577 (truncating int math)
        assert_eq!(c.x, 1920 - 577);
        assert_eq!(c.y, 0);
    }

    #[test]
    fn crop_bottom_left_720p() {
        let c = pick_facecam_crop(1280, 720, FacecamCorner::BottomLeft);
        assert_eq!(c.h, 684); // 95% of 720
        assert_eq!(c.w, 684 * 9 / 16);
        assert_eq!(c.x, 0);
        assert_eq!(c.y, 720 - 684);
    }

    #[test]
    fn crop_clamps_when_source_too_narrow() {
        // 480-wide source can't fit a 9:16 vertical of height 95% × 480
        let c = pick_facecam_crop(480, 480, FacecamCorner::TopRight);
        assert!(c.w <= 480);
        assert!(c.x + c.w <= 480);
        assert!(c.h <= 480);
        assert!(c.y + c.h <= 480);
    }

    #[test]
    fn score_empty_bytes_is_zero() {
        let (b, v, r) = score_bytes(&[]);
        assert_eq!(b, 0);
        assert_eq!(v, 0);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn score_high_variance_outranks_uniform() {
        // 1024 uniform bytes vs 1024 noisy bytes — noisy should rank
        // higher because variance dominates the score formula.
        let uniform = vec![128u8; 1024];
        let noisy: Vec<u8> = (0..1024u32).map(|i| ((i * 17) % 256) as u8).collect();
        let (_, _, score_uniform) = score_bytes(&uniform);
        let (_, _, score_noisy) = score_bytes(&noisy);
        assert!(
            score_noisy > score_uniform,
            "noisy {score_noisy} should > uniform {score_uniform}"
        );
    }

    #[test]
    fn score_larger_jpeg_outranks_smaller_at_same_variance() {
        // Same byte pattern, different lengths — the log-bytes term
        // breaks the tie in favour of the longer (richer) image.
        let pattern: Vec<u8> = (0..256u32).map(|i| (i % 256) as u8).collect();
        let small = pattern.clone();
        let big: Vec<u8> = pattern.iter().cycle().take(16384).copied().collect();
        let (_, _, score_small) = score_bytes(&small);
        let (_, _, score_big) = score_bytes(&big);
        assert!(
            score_big > score_small,
            "big {score_big} should > small {score_small}"
        );
    }

    #[test]
    fn normalise_scores_clamps_to_unit_range() {
        let mut set = vec![cand(0.0, 5.0), cand(10.0, 2.5), cand(20.0, 0.0)];
        normalise_scores(&mut set);
        assert!((set[0].score - 1.0).abs() < 1e-5);
        assert!((set[1].score - 0.5).abs() < 1e-5);
        assert_eq!(set[2].score, 0.0);
    }

    #[test]
    fn normalise_scores_no_op_when_all_zero() {
        let mut set = vec![cand(0.0, 0.0), cand(10.0, 0.0)];
        normalise_scores(&mut set);
        assert!(set.iter().all(|c| c.score == 0.0));
    }
}
