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

/// Styled caption export options — surfaces in the Editor "Styled
/// subtitles" panel and persists with the recording. Designed for the
/// YouTube Shorts / TikTok / Reels cut where styled captions drive
/// watch-time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssStyle {
    /// Title in the ASS header.
    pub title: String,
    /// Default font face. Picks one that the player is likely to have;
    /// system fallback resolves the missing-font case.
    pub font: String,
    /// Default font size in script-resolution points.
    pub font_size: u32,
    /// Outline thickness in pixels. Heavy outline = readable over busy
    /// video; lower outline = aesthetic.
    pub outline_px: f32,
    /// Drop-shadow distance in pixels.
    pub shadow_px: f32,
    /// 1..=9 numpad-style alignment. 2 = bottom-centre (default).
    pub alignment: u8,
    /// Bottom margin in pixels. Shorts / Reels usually want 80-120 to
    /// avoid the UI overlays.
    pub margin_v_px: u32,
    /// Optional per-speaker colour map — speaker label → ASS BGR hex
    /// (e.g. "FFFFFF" for white, "00FFFF" for yellow). When present,
    /// each cue gets the speaker's colour via a `{\c}` override.
    #[serde(default)]
    pub speaker_colors: std::collections::BTreeMap<String, String>,
}

impl Default for AssStyle {
    fn default() -> Self {
        Self {
            title: "StriVo styled captions".into(),
            font: "Inter".into(),
            font_size: 56,
            outline_px: 2.0,
            shadow_px: 1.5,
            alignment: 2,
            margin_v_px: 80,
            speaker_colors: Default::default(),
        }
    }
}

/// One word with its sub-segment timing — drives karaoke-style `\k`
/// highlight in ASS dialogue lines. Optional; absent karaoke = the
/// whole cue text appears at once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordTiming {
    pub text: String,
    pub start_sec: f32,
    pub end_sec: f32,
}

/// A caption cue enriched with optional per-word timing for karaoke
/// rendering. The plain SRT/VTT/TXT paths still work off the simpler
/// [`Segment`] shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaraokeSegment {
    pub start_sec: f32,
    pub end_sec: f32,
    pub speaker: Option<String>,
    pub words: Vec<WordTiming>,
}

/// Format an ASS time as `H:MM:SS.cc` (centiseconds, single-digit hour).
fn fmt_ass_time(sec: f32) -> String {
    let cs = (sec.max(0.0) * 100.0).round() as u64;
    let h = cs / 360_000;
    let m = (cs / 6000) % 60;
    let s = (cs / 100) % 60;
    let c = cs % 100;
    format!("{h}:{m:02}:{s:02}.{c:02}")
}

/// Emit an Advanced SubStation Alpha (.ass) caption file. ASS supports
/// per-cue colour, outline, drop shadow, positioning, and the karaoke
/// `\k` highlight tag — everything you need for YouTube Shorts /
/// TikTok / Reels-style "bold readable captions".
///
/// Pass a `karaoke` list when word-level timing is available; the cue
/// for each segment is built from the matching `KaraokeSegment` if its
/// start/end roughly aligns (within 0.05s). Cues with no karaoke fall
/// back to the segment's plain text.
pub fn to_ass(
    segments: &[Segment],
    style: &AssStyle,
    karaoke: &[KaraokeSegment],
) -> String {
    let mut out = String::new();
    out.push_str("[Script Info]\n");
    out.push_str(&format!("Title: {}\n", ass_escape(&style.title)));
    out.push_str("ScriptType: v4.00+\n");
    out.push_str("PlayResX: 1920\nPlayResY: 1080\n");
    out.push_str("ScaledBorderAndShadow: yes\n\n");
    out.push_str("[V4+ Styles]\n");
    out.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    // ASS colours are &HAABBGGRR (alpha + BGR). White text, black
    // outline, 50% black shadow — classic high-contrast preset.
    out.push_str(&format!(
        "Style: Default,{font},{size},&H00FFFFFF,&H000000FF,&H00000000,&H7F000000,1,0,0,0,100,100,0,0,1,{outline:.2},{shadow:.2},{align},20,20,{marginv},1\n\n",
        font = style.font,
        size = style.font_size,
        outline = style.outline_px,
        shadow = style.shadow_px,
        align = style.alignment,
        marginv = style.margin_v_px,
    ));
    out.push_str("[Events]\n");
    out.push_str("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
    // Build a fast lookup so we can match karaoke entries by their
    // (rounded) start time without an O(n*m) scan.
    let kar_by_start: std::collections::HashMap<u32, &KaraokeSegment> = karaoke
        .iter()
        .map(|k| ((k.start_sec * 100.0).round() as u32, k))
        .collect();
    for s in segments {
        let key = (s.start_sec * 100.0).round() as u32;
        let kar = kar_by_start.get(&key).filter(|k| {
            (k.end_sec - s.end_sec).abs() < 0.05
        });
        let speaker_override = match &s.speaker {
            Some(spk) => style
                .speaker_colors
                .get(spk)
                .map(|hex| format!("{{\\c&H00{hex}&}}", hex = hex.to_uppercase())),
            None => None,
        }
        .unwrap_or_default();
        let text = if let Some(k) = kar {
            karaoke_text(k, &speaker_override)
        } else {
            format!("{}{}", speaker_override, ass_escape(s.text.trim()))
        };
        let line = format!(
            "Dialogue: 0,{start},{end},Default,,0,0,0,,{text}\n",
            start = fmt_ass_time(s.start_sec),
            end = fmt_ass_time(s.end_sec),
        );
        out.push_str(&line);
    }
    out
}

fn karaoke_text(seg: &KaraokeSegment, color_prefix: &str) -> String {
    // ASS \k takes centisecond durations.
    let mut out = String::new();
    out.push_str(color_prefix);
    for w in &seg.words {
        let cs = ((w.end_sec - w.start_sec).max(0.0) * 100.0).round() as u32;
        out.push_str(&format!("{{\\k{cs}}}{} ", ass_escape(w.text.trim())));
    }
    out.trim_end().to_string()
}

/// Minimal ASS escape — guard the two characters that break dialogue
/// parsing: `\N` is the line-break sequence (we keep newlines literal
/// by replacing them), and `{` opens override blocks.
fn ass_escape(s: &str) -> String {
    s.replace('\n', "\\N").replace('{', "\\{").replace('}', "\\}")
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

    fn kar(start: f32, end: f32, spk: Option<&str>, words: &[(&str, f32, f32)]) -> KaraokeSegment {
        KaraokeSegment {
            start_sec: start,
            end_sec: end,
            speaker: spk.map(|s| s.into()),
            words: words.iter().map(|(t, s, e)| WordTiming {
                text: (*t).into(), start_sec: *s, end_sec: *e,
            }).collect(),
        }
    }

    #[test]
    fn ass_header_carries_style_knobs() {
        let mut style = AssStyle::default();
        style.font = "Helvetica".into();
        style.font_size = 72;
        style.outline_px = 3.0;
        style.shadow_px = 2.0;
        style.margin_v_px = 120;
        let s = to_ass(&[], &style, &[]);
        assert!(s.contains("Title: StriVo styled captions"));
        assert!(s.contains("Helvetica,72,"));
        assert!(s.contains(",3.00,2.00,2,20,20,120,"));
        assert!(s.contains("PlayResX: 1920"));
    }

    #[test]
    fn ass_dialogue_line_emits_centisecond_timestamps() {
        let segs = vec![seg(1.0, 5.5, "hello world", None)];
        let s = to_ass(&segs, &AssStyle::default(), &[]);
        assert!(s.contains("Dialogue: 0,0:00:01.00,0:00:05.50,Default,,0,0,0,,hello world"));
    }

    #[test]
    fn ass_per_speaker_color_overrides_cue() {
        let mut style = AssStyle::default();
        style.speaker_colors.insert("Alice".into(), "00FFFF".into()); // yellow (BGR)
        let segs = vec![seg(0.0, 1.0, "hi", Some("Alice"))];
        let s = to_ass(&segs, &style, &[]);
        // Cue should carry the colour override before the text.
        assert!(s.contains(r"{\c&H0000FFFF&}hi"));
    }

    #[test]
    fn ass_karaoke_emits_per_word_k_tags() {
        let segs = vec![seg(0.0, 2.0, "hello world", None)];
        let k = vec![kar(0.0, 2.0, None, &[("hello", 0.0, 0.5), ("world", 0.5, 2.0)])];
        let s = to_ass(&segs, &AssStyle::default(), &k);
        // hello = 50cs, world = 150cs.
        assert!(s.contains(r"{\k50}hello"));
        assert!(s.contains(r"{\k150}world"));
    }

    #[test]
    fn ass_karaoke_falls_back_to_plain_text_on_mismatch() {
        let segs = vec![seg(0.0, 2.0, "hello world", None)];
        // Karaoke entry with mismatched end time → not used.
        let k = vec![kar(0.0, 3.0, None, &[("hello", 0.0, 0.5)])];
        let s = to_ass(&segs, &AssStyle::default(), &k);
        assert!(s.contains(",hello world"));
        assert!(!s.contains(r"{\k"));
    }

    #[test]
    fn ass_escape_handles_braces_and_newlines() {
        let segs = vec![seg(0.0, 1.0, "use {0} and\n{1}", None)];
        let s = to_ass(&segs, &AssStyle::default(), &[]);
        // Newline → \N; braces → escaped.
        assert!(s.contains(r"use \{0\} and\N\{1\}"));
    }

    #[test]
    fn ass_empty_segments_still_produce_valid_header() {
        let s = to_ass(&[], &AssStyle::default(), &[]);
        assert!(s.contains("[Script Info]"));
        assert!(s.contains("[V4+ Styles]"));
        assert!(s.contains("[Events]"));
        // No dialogue lines.
        assert!(!s.contains("\nDialogue:"));
    }

    #[test]
    fn ass_combines_color_and_karaoke_on_same_cue() {
        let mut style = AssStyle::default();
        style.speaker_colors.insert("Bob".into(), "0080FF".into());
        let segs = vec![seg(0.0, 1.0, "go go", Some("Bob"))];
        let k = vec![kar(0.0, 1.0, Some("Bob"), &[("go", 0.0, 0.5), ("go", 0.5, 1.0)])];
        let s = to_ass(&segs, &style, &k);
        assert!(s.contains(r"{\c&H000080FF&}{\k50}go {\k50}go"));
    }

    #[test]
    fn fmt_ass_time_handles_centisecond_rounding() {
        assert_eq!(fmt_ass_time(0.0), "0:00:00.00");
        assert_eq!(fmt_ass_time(3666.789), "1:01:06.79");
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
