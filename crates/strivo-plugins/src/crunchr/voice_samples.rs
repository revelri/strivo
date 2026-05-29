//! Voice-sample slicing for the Speaker Editor modal.
//!
//! After a diarized transcription completes we want a short, representative
//! audio clip per speaker so the user can audition each row in the modal
//! before renaming labels. The clip is cached on disk and played via the
//! existing mpv plumbing (`PluginAction::PlayFile`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::crunchr::types::Segment;

/// Window length around a speaker's pivot segment, in seconds. Long enough to
/// identify a voice, short enough to keep mpv launches snappy and the cache tiny.
const SAMPLE_SECS: f64 = 4.0;

/// Minimum / maximum duration of a segment we'll pick as the pivot. Avoids
/// monosyllables and long monologues where the start of the segment may overlap
/// crosstalk from the previous speaker.
const PIVOT_MIN_SECS: f64 = 2.0;
const PIVOT_MAX_SECS: f64 = 8.0;

/// One audition clip ready to play in the Speaker Editor modal.
#[derive(Debug, Clone)]
pub struct SpeakerSample {
    pub speaker: String,
    pub sample_path: PathBuf,
}

/// Cheap filesystem-safe slug for the speaker label (e.g. "Speaker 0" -> "speaker_0").
/// Exposed so the Speaker Editor modal can resolve cached sample paths the
/// same way the slicer writes them.
pub fn slugify(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "speaker".to_string()
    } else {
        trimmed
    }
}

/// Pick the best pivot segment for each unique speaker label.
///
/// Pulled out of `slice_samples` so the picking logic is unit-testable without
/// touching ffmpeg or the filesystem.
pub fn pick_pivots(segments: &[Segment]) -> Vec<(&str, &Segment)> {
    let mut best: HashMap<&str, &Segment> = HashMap::new();
    for s in segments {
        let Some(spk) = s.speaker.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        let dur = s.end_sec - s.start_sec;
        if !dur.is_finite() || dur <= 0.0 {
            continue;
        }
        // Prefer segments inside [PIVOT_MIN_SECS, PIVOT_MAX_SECS]. If none of
        // a speaker's segments fall in that band, fall back to the longest.
        let prefer = (PIVOT_MIN_SECS..=PIVOT_MAX_SECS).contains(&dur);
        match best.get(spk) {
            None => {
                best.insert(spk, s);
            }
            Some(cur) => {
                let cur_dur = cur.end_sec - cur.start_sec;
                let cur_prefer = (PIVOT_MIN_SECS..=PIVOT_MAX_SECS).contains(&cur_dur);
                let replace = match (cur_prefer, prefer) {
                    (false, true) => true,
                    (true, false) => false,
                    _ => dur > cur_dur,
                };
                if replace {
                    best.insert(spk, s);
                }
            }
        }
    }
    let mut out: Vec<(&str, &Segment)> = best.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// Slice a ~4-second voice sample per speaker into `out_dir`. Idempotent —
/// existing files are left untouched. Returns one [`SpeakerSample`] per
/// speaker we managed to slice (or that already had a cached clip).
///
/// `video_path` should be the recording's `.mkv` (with audio); ffmpeg pulls
/// audio out of it directly so we don't depend on the temporary WAV.
pub async fn slice_samples(
    video_path: &Path,
    segments: &[Segment],
    out_dir: &Path,
) -> anyhow::Result<Vec<SpeakerSample>> {
    if !video_path.exists() {
        anyhow::bail!(
            "voice_samples: source video missing: {}",
            video_path.display()
        );
    }
    std::fs::create_dir_all(out_dir)?;
    let pivots = pick_pivots(segments);
    let mut out = Vec::with_capacity(pivots.len());

    for (spk, seg) in pivots {
        let slug = slugify(spk);
        let dest = out_dir.join(format!("{slug}.wav"));
        if !dest.exists() {
            // Centre the SAMPLE_SECS window on the middle of the pivot segment.
            let mid = (seg.start_sec + seg.end_sec) * 0.5;
            let start = (mid - SAMPLE_SECS * 0.5).max(0.0);
            let status = tokio::process::Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "warning",
                    "-ss",
                    &format!("{start:.3}"),
                    "-t",
                    &format!("{SAMPLE_SECS:.3}"),
                    "-i",
                ])
                .arg(video_path)
                .args([
                    "-vn",
                    "-ac",
                    "1",
                    "-ar",
                    "16000",
                    "-acodec",
                    "pcm_s16le",
                    "-y",
                ])
                .arg(&dest)
                .status()
                .await?;
            if !status.success() {
                tracing::warn!("voice_samples: ffmpeg failed for {spk} ({status}); skipping");
                continue;
            }
        }
        out.push(SpeakerSample {
            speaker: spk.to_string(),
            sample_path: dest,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(speaker: &str, start: f64, end: f64) -> Segment {
        Segment {
            index: 0,
            start_sec: start,
            end_sec: end,
            text: String::new(),
            speaker: Some(speaker.to_string()),
            confidence: None,
            words: None,
        }
    }

    #[test]
    fn slug_filesystem_safe() {
        assert_eq!(slugify("Speaker 0"), "speaker_0");
        assert_eq!(slugify("Alice / Bob"), "alice_bob");
        assert_eq!(slugify("    "), "speaker");
        assert_eq!(slugify("José"), "jos");
    }

    #[test]
    fn pivot_prefers_in_band_segment_over_longest() {
        // Two segments for "A": a 30 s monologue (outside band) and a 5 s
        // utterance (in band). The 5 s one should win.
        let segs = vec![
            seg("A", 0.0, 30.0),
            seg("A", 100.0, 105.0),
            seg("B", 50.0, 53.0),
        ];
        let pivots = pick_pivots(&segs);
        assert_eq!(pivots.len(), 2);
        let by_spk: std::collections::HashMap<_, _> = pivots
            .into_iter()
            .map(|(spk, s)| (spk.to_string(), s.start_sec))
            .collect();
        assert_eq!(by_spk["A"], 100.0); // in-band pick
        assert_eq!(by_spk["B"], 50.0);
    }

    #[test]
    fn pivot_falls_back_to_longest_when_no_in_band() {
        // All A segments are too short. Pick the longest (which is still < min).
        let segs = vec![
            seg("A", 0.0, 0.5),
            seg("A", 5.0, 6.5), // 1.5 s — under min but longest
            seg("A", 10.0, 10.8),
        ];
        let pivots = pick_pivots(&segs);
        assert_eq!(pivots.len(), 1);
        assert_eq!(pivots[0].1.start_sec, 5.0);
    }

    #[test]
    fn ignores_unspeakered_or_invalid_segments() {
        let segs = vec![
            Segment {
                index: 0,
                start_sec: 0.0,
                end_sec: 5.0,
                text: String::new(),
                speaker: None,
                confidence: None,
                words: None,
            },
            seg("A", 10.0, 8.0), // negative duration, ignored
            seg("A", 20.0, 25.0),
        ];
        let pivots = pick_pivots(&segs);
        assert_eq!(pivots.len(), 1);
        assert_eq!(pivots[0].1.start_sec, 20.0);
    }
}
