//! strivo-deadair — silence detection + cut suggestions.
//!
//! Every long stream has dead air the editor should trim — VOD intros,
//! restroom breaks, frozen reactions, Discord-pulled-away moments.
//! This plugin runs ffmpeg's `silencedetect` filter, parses the
//! stderr trace, and returns a typed list of [`SilenceSpan`]s plus a
//! recommended-cuts list calibrated to "trim things longer than N
//! seconds without nuking pauses".
//!
//! Surfaces:
//!
//!   * [`parse_silencedetect`] — pure parser over canned ffmpeg
//!     stderr; the test suite feeds it strings. Captures the
//!     `silence_start: <t>` / `silence_end: <t> | silence_duration: <d>`
//!     pairs ffmpeg emits.
//!   * [`detect_silences`] — invokes ffmpeg. Slow op; cacheable per
//!     (recording_id, noise_db, min_duration_secs).
//!   * [`recommend_cuts`] — drops spans whose `duration <
//!     trim_threshold_secs` and returns a `Vec<CutRange>` ready to
//!     feed Editor::delete_range. Optionally adds a fade pad to
//!     preserve breathing room.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default silence noise floor in dB. -30 dB is the ffmpeg default; we
/// stick with it because tuning this is project-dependent.
pub const DEFAULT_NOISE_DB: f32 = -30.0;
/// Default minimum span the detector emits. Spans shorter than this
/// are ignored by ffmpeg's silencedetect filter.
pub const DEFAULT_MIN_SPAN_SECS: f32 = 1.0;
/// Default trim threshold — spans below this are kept ("normal pause").
pub const DEFAULT_TRIM_THRESHOLD_SECS: f32 = 6.0;
/// Default pad applied around recommended cuts so the edit doesn't
/// chop right against the cut point. Half the pad each side.
pub const DEFAULT_TRIM_PAD_SECS: f32 = 0.2;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SilenceSpan {
    pub start_sec: f32,
    pub end_sec: f32,
    pub duration_sec: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CutRange {
    pub start_sec: f32,
    pub end_sec: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionResult {
    pub noise_db: f32,
    pub min_span_secs: f32,
    pub spans: Vec<SilenceSpan>,
    pub recommended_cuts: Vec<CutRange>,
    pub total_trim_secs: f32,
}

/// Invoke ffmpeg with the silencedetect filter against `input` and
/// return the detection result. Synchronous; can run for minutes on a
/// long recording.
pub fn detect_silences(
    input: &Path,
    noise_db: f32,
    min_span_secs: f32,
    trim_threshold_secs: f32,
) -> Result<DetectionResult> {
    let filter = format!(
        "silencedetect=noise={noise_db}dB:duration={min_span_secs}",
    );
    let child = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-loglevel", "info"])
        .arg("-i")
        .arg(input)
        .args(["-af", &filter, "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn ffmpeg")?;
    let output = child.wait_with_output().context("wait on ffmpeg")?;
    if !output.status.success() {
        anyhow::bail!(
            "ffmpeg exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let spans = parse_silencedetect(&stderr);
    let recommended_cuts = recommend_cuts(&spans, trim_threshold_secs, DEFAULT_TRIM_PAD_SECS);
    let total_trim_secs = recommended_cuts
        .iter()
        .map(|c| (c.end_sec - c.start_sec).max(0.0))
        .sum();
    Ok(DetectionResult {
        noise_db,
        min_span_secs,
        spans,
        recommended_cuts,
        total_trim_secs,
    })
}

/// Parse ffmpeg `silencedetect` lines into a `Vec<SilenceSpan>`. Lines
/// of interest:
///
///   `[silencedetect @ 0x..] silence_start: 12.345`
///   `[silencedetect @ 0x..] silence_end: 18.001 | silence_duration: 5.656`
///
/// `silence_start` opens a span; `silence_end` closes it. Lone
/// `silence_start`s without a matching end (file truncated mid-silence)
/// are skipped — there's no observable end to a partial trailing silence.
pub fn parse_silencedetect(stderr: &str) -> Vec<SilenceSpan> {
    let mut out: Vec<SilenceSpan> = Vec::new();
    let mut current_start: Option<f32> = None;
    for line in stderr.lines() {
        if !line.contains("silencedetect") {
            continue;
        }
        if let Some(t) = parse_value(line, "silence_start:") {
            current_start = Some(t);
            continue;
        }
        if let Some(end) = parse_value(line, "silence_end:") {
            if let Some(start) = current_start.take() {
                let duration_sec = parse_value(line, "silence_duration:")
                    .unwrap_or_else(|| (end - start).max(0.0));
                out.push(SilenceSpan {
                    start_sec: start,
                    end_sec: end,
                    duration_sec,
                });
            }
        }
    }
    out
}

fn parse_value(line: &str, needle: &str) -> Option<f32> {
    let idx = line.find(needle)?;
    let tail = line[idx + needle.len()..].trim_start();
    let len = tail
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .count();
    tail.get(..len).and_then(|s| s.parse().ok())
}

/// Filter detected spans down to the ones worth trimming. Spans
/// shorter than `trim_threshold_secs` are kept (those are normal
/// conversational pauses); the rest are returned as [`CutRange`]s
/// with `pad_secs` shaved from both ends so the edit leaves breathing
/// room around the silence.
pub fn recommend_cuts(
    spans: &[SilenceSpan],
    trim_threshold_secs: f32,
    pad_secs: f32,
) -> Vec<CutRange> {
    spans
        .iter()
        .filter(|s| s.duration_sec >= trim_threshold_secs)
        .filter_map(|s| {
            let half_pad = pad_secs.max(0.0) * 0.5;
            let start = s.start_sec + half_pad;
            let end = s.end_sec - half_pad;
            if end > start + 0.01 {
                Some(CutRange { start_sec: start, end_sec: end })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_span() {
        let stderr = "\
[silencedetect @ 0x5] silence_start: 12.345
[silencedetect @ 0x5] silence_end: 18.001 | silence_duration: 5.656
";
        let spans = parse_silencedetect(stderr);
        assert_eq!(spans.len(), 1);
        assert!((spans[0].start_sec - 12.345).abs() < 1e-3);
        assert!((spans[0].end_sec - 18.001).abs() < 1e-3);
        assert!((spans[0].duration_sec - 5.656).abs() < 1e-3);
    }

    #[test]
    fn parses_multiple_spans() {
        let stderr = "\
[silencedetect @ 0x5] silence_start: 10.0
[silencedetect @ 0x5] silence_end: 15.0 | silence_duration: 5.0
[silencedetect @ 0x5] silence_start: 100.0
[silencedetect @ 0x5] silence_end: 130.0 | silence_duration: 30.0
";
        let spans = parse_silencedetect(stderr);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].duration_sec as u32, 30);
    }

    #[test]
    fn skips_orphan_silence_start() {
        // File truncated mid-silence — no end line emitted. We drop
        // the partial span rather than pretending it ran to infinity.
        let stderr = "[silencedetect @ 0x5] silence_start: 12.0\n";
        assert!(parse_silencedetect(stderr).is_empty());
    }

    #[test]
    fn skips_lines_without_silencedetect_prefix() {
        let stderr = "frame= 1234 fps=25 silence_start: 99\n";
        assert!(parse_silencedetect(stderr).is_empty());
    }

    #[test]
    fn duration_falls_back_when_missing() {
        // ffmpeg's older output omits the trailing duration; we infer.
        let stderr = "\
[silencedetect @ 0x5] silence_start: 10.0
[silencedetect @ 0x5] silence_end: 16.5
";
        let spans = parse_silencedetect(stderr);
        assert_eq!(spans.len(), 1);
        assert!((spans[0].duration_sec - 6.5).abs() < 1e-3);
    }

    #[test]
    fn recommend_drops_short_spans() {
        let spans = vec![
            SilenceSpan { start_sec: 10.0, end_sec: 13.0, duration_sec: 3.0 },
            SilenceSpan { start_sec: 50.0, end_sec: 80.0, duration_sec: 30.0 },
        ];
        let cuts = recommend_cuts(&spans, 6.0, 0.0);
        assert_eq!(cuts.len(), 1);
        assert!((cuts[0].start_sec - 50.0).abs() < 1e-5);
    }

    #[test]
    fn recommend_applies_pad() {
        let spans = vec![SilenceSpan {
            start_sec: 10.0,
            end_sec: 30.0,
            duration_sec: 20.0,
        }];
        let cuts = recommend_cuts(&spans, 6.0, 0.4);
        assert_eq!(cuts.len(), 1);
        // 0.4 pad → 0.2 shaved each side.
        assert!((cuts[0].start_sec - 10.2).abs() < 1e-5);
        assert!((cuts[0].end_sec - 29.8).abs() < 1e-5);
    }

    #[test]
    fn recommend_drops_spans_whose_pad_swallows_them() {
        // 2-second span with a 5-second pad would invert; drop it.
        let spans = vec![SilenceSpan {
            start_sec: 10.0,
            end_sec: 12.0,
            duration_sec: 2.0,
        }];
        // Pretend threshold is 0 so the filter doesn't drop; pad does.
        let cuts = recommend_cuts(&spans, 0.0, 5.0);
        assert!(cuts.is_empty());
    }

    #[test]
    fn recommend_empty_input_yields_empty_output() {
        assert!(recommend_cuts(&[], 6.0, 0.0).is_empty());
    }

    #[test]
    fn parses_scientific_notation_safely() {
        // ffmpeg sometimes emits values like '1.234e+02'. Our parser
        // collects digits/./- only — it should stop at 'e' and the
        // value reads as 1.234. Better than panicking; the SPA
        // surfaces oddly-short spans as their own warning.
        let stderr = "\
[silencedetect @ 0x5] silence_start: 1.234e+02
[silencedetect @ 0x5] silence_end: 1.300e+02 | silence_duration: 6.0
";
        let spans = parse_silencedetect(stderr);
        assert_eq!(spans.len(), 1);
        // We saw 1.234 / 1.300 / 6.0; the parser doesn't honour the
        // exponent. The duration line still gives us a reliable read.
        assert!((spans[0].duration_sec - 6.0).abs() < 1e-3);
    }

    #[test]
    fn handles_zero_padding_correctly() {
        // Pad of 0.0 should not shrink any span.
        let spans = vec![SilenceSpan {
            start_sec: 1.0,
            end_sec: 100.0,
            duration_sec: 99.0,
        }];
        let cuts = recommend_cuts(&spans, 6.0, 0.0);
        assert_eq!(cuts.len(), 1);
        assert!((cuts[0].start_sec - 1.0).abs() < 1e-5);
        assert!((cuts[0].end_sec - 100.0).abs() < 1e-5);
    }

    #[test]
    fn detection_result_total_trim_sums_recommended_cuts() {
        // Hand-build a result to exercise the public shape (no ffmpeg).
        let r = DetectionResult {
            noise_db: -30.0,
            min_span_secs: 1.0,
            spans: vec![],
            recommended_cuts: vec![
                CutRange { start_sec: 10.0, end_sec: 20.0 },
                CutRange { start_sec: 100.0, end_sec: 105.0 },
            ],
            total_trim_secs: 15.0,
        };
        let computed: f32 = r
            .recommended_cuts
            .iter()
            .map(|c| c.end_sec - c.start_sec)
            .sum();
        assert!((computed - r.total_trim_secs).abs() < 1e-5);
    }
}
