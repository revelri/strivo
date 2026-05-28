//! strivo-captions — caption file generation + translation interface.
//!
//! Two surfaces for the DAW-vision:
//!
//!   1. Format conversion: take Crunchr transcript segments
//!      (start_sec, end_sec, speaker, text) and emit publishable
//!      caption files — SRT (universal), VTT (web standard, with
//!      `<v Speaker>` tags), TXT (paste-into-show-notes).
//!   2. Translation: a [`Translator`] trait so backends are pluggable.
//!      The default [`IdentityTranslator`] is a no-op pass-through; a
//!      future iteration plugs in NLLB / Argos / OpenAI as needed.
//!      [`apply_translation`] takes a list of segments and a translator
//!      and returns the translated copy ready to format.
//!
//! Pure data; no IO; runs in tests without fixtures.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start_sec: f32,
    pub end_sec: f32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

/// Pluggable translation backend. Implementations may be sync or
/// wrap an async runtime — the trait keeps things synchronous so the
/// formatter functions stay testable without tokio.
pub trait Translator: Send + Sync {
    /// Source language hint (e.g. `"en"`). May be empty when unknown.
    fn source_lang(&self) -> &str;
    /// Target language code (e.g. `"es"`, `"pt-BR"`, `"ja"`).
    fn target_lang(&self) -> &str;
    /// Translate a single string. Implementations may chunk and batch
    /// however they like, as long as the contract holds.
    fn translate(&self, text: &str) -> anyhow::Result<String>;
}

/// No-op translator — returns the input verbatim. Useful as a default,
/// and as the test target for the format-conversion helpers.
pub struct IdentityTranslator;

impl Translator for IdentityTranslator {
    fn source_lang(&self) -> &str {
        "en"
    }
    fn target_lang(&self) -> &str {
        "en"
    }
    fn translate(&self, text: &str) -> anyhow::Result<String> {
        Ok(text.to_string())
    }
}

/// Apply a translator across a segment list. Failures on individual
/// segments are surfaced — the caller decides whether to retry or
/// continue with a partial result.
pub fn apply_translation(
    segments: &[Segment],
    t: &dyn Translator,
) -> anyhow::Result<Vec<Segment>> {
    let mut out = Vec::with_capacity(segments.len());
    for s in segments {
        out.push(Segment {
            start_sec: s.start_sec,
            end_sec: s.end_sec,
            text: t.translate(&s.text)?,
            speaker: s.speaker.clone(),
        });
    }
    Ok(out)
}

/// Format a duration as `HH:MM:SS,mmm` (SRT) or `HH:MM:SS.mmm` (VTT).
fn fmt_time(sec: f32, sep: char) -> String {
    let ms = (sec.max(0.0) * 1000.0).round() as u64;
    let h = ms / 3_600_000;
    let m = (ms / 60_000) % 60;
    let s = (ms / 1000) % 60;
    let r = ms % 1000;
    format!("{h:02}:{m:02}:{s:02}{sep}{r:03}")
}

/// Emit a SubRip (.srt) caption file. Numbered entries, comma-decimal
/// timestamps, speaker label prefixed when present.
pub fn to_srt(segments: &[Segment]) -> String {
    let mut out = String::new();
    for (i, s) in segments.iter().enumerate() {
        out.push_str(&format!("{}\n", i + 1));
        out.push_str(&format!(
            "{} --> {}\n",
            fmt_time(s.start_sec, ','),
            fmt_time(s.end_sec, ',')
        ));
        let line = match &s.speaker {
            Some(spk) if !spk.is_empty() => format!("[{spk}] {}", s.text.trim()),
            _ => s.text.trim().to_string(),
        };
        out.push_str(&line);
        out.push_str("\n\n");
    }
    out
}

/// Emit a WebVTT (.vtt) file. VTT uses `.` for the decimal separator
/// and supports `<v Speaker>text` voice tags which players render with
/// styling. Header is the mandatory `WEBVTT` line.
pub fn to_vtt(segments: &[Segment]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for (i, s) in segments.iter().enumerate() {
        out.push_str(&format!("{}\n", i + 1));
        out.push_str(&format!(
            "{} --> {}\n",
            fmt_time(s.start_sec, '.'),
            fmt_time(s.end_sec, '.')
        ));
        let line = match &s.speaker {
            Some(spk) if !spk.is_empty() => format!("<v {spk}>{}", s.text.trim()),
            _ => s.text.trim().to_string(),
        };
        out.push_str(&line);
        out.push_str("\n\n");
    }
    out
}

/// Plain-text export — drops timestamps + tags, useful for show-notes
/// drafts or to feed a separate summarisation pipeline. Keeps speaker
/// labels because they read well in markdown.
pub fn to_txt(segments: &[Segment]) -> String {
    let mut out = String::new();
    for s in segments {
        match &s.speaker {
            Some(spk) if !spk.is_empty() => out.push_str(&format!("{spk}: ")),
            _ => {}
        }
        out.push_str(s.text.trim());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f32, end: f32, text: &str, spk: Option<&str>) -> Segment {
        Segment {
            start_sec: start,
            end_sec: end,
            text: text.into(),
            speaker: spk.map(|s| s.into()),
        }
    }

    struct UpperTranslator;
    impl Translator for UpperTranslator {
        fn source_lang(&self) -> &str { "en" }
        fn target_lang(&self) -> &str { "en-shout" }
        fn translate(&self, text: &str) -> anyhow::Result<String> { Ok(text.to_uppercase()) }
    }

    #[test]
    fn srt_emits_numbered_entries_with_comma_decimal() {
        let segs = vec![
            seg(1.0, 5.5, "hello world", None),
            seg(6.0, 10.123, "second line", Some("Alice")),
        ];
        let s = to_srt(&segs);
        assert!(s.contains("00:00:01,000 --> 00:00:05,500"), "got:\n{s}");
        assert!(s.contains("00:00:06,000 --> 00:00:10,123"));
        assert!(s.contains("1\n"));
        assert!(s.contains("2\n"));
        assert!(s.contains("hello world"));
        assert!(s.contains("[Alice] second line"));
    }

    #[test]
    fn vtt_emits_header_dot_decimal_and_voice_tag() {
        let segs = vec![seg(0.0, 3.0, "ok", Some("Bob"))];
        let s = to_vtt(&segs);
        assert!(s.starts_with("WEBVTT\n"));
        assert!(s.contains("00:00:00.000 --> 00:00:03.000"));
        assert!(s.contains("<v Bob>ok"));
    }

    #[test]
    fn txt_drops_timestamps_keeps_speakers() {
        let segs = vec![
            seg(0.0, 1.0, "hi", Some("Alice")),
            seg(1.0, 2.0, "bye", None),
        ];
        let s = to_txt(&segs);
        assert!(s.contains("Alice: hi"));
        assert!(s.contains("bye"));
        assert!(!s.contains("00:00"));
    }

    #[test]
    fn fmt_time_handles_hours() {
        // 1h 2m 3s + 456ms
        let t = 3600.0 + 120.0 + 3.0 + 0.456;
        assert_eq!(fmt_time(t, ','), "01:02:03,456");
    }

    #[test]
    fn fmt_time_clamps_negative_to_zero() {
        assert_eq!(fmt_time(-5.0, ','), "00:00:00,000");
    }

    #[test]
    fn identity_translator_round_trips_text() {
        let segs = vec![seg(0.0, 1.0, "hola", None)];
        let out = apply_translation(&segs, &IdentityTranslator).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "hola");
    }

    #[test]
    fn upper_translator_transforms_each_segment() {
        let segs = vec![
            seg(0.0, 1.0, "hello", None),
            seg(1.0, 2.0, "world", Some("Alice")),
        ];
        let out = apply_translation(&segs, &UpperTranslator).unwrap();
        assert_eq!(out[0].text, "HELLO");
        assert_eq!(out[1].text, "WORLD");
        // Speaker should pass through untranslated.
        assert_eq!(out[1].speaker.as_deref(), Some("Alice"));
    }

    #[test]
    fn empty_segments_yield_empty_outputs() {
        assert_eq!(to_srt(&[]), "");
        assert_eq!(to_vtt(&[]), "WEBVTT\n\n");
        assert_eq!(to_txt(&[]), "");
    }

    #[test]
    fn trims_whitespace_around_text() {
        let segs = vec![seg(0.0, 1.0, "   hello   ", None)];
        let s = to_srt(&segs);
        // The line should be "hello", not "   hello   "
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines[2], "hello");
    }
}
