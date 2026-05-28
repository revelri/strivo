//! strivo-reuse — cross-format publish-queue drafter.
//!
//! Streamers turn one stream into N pieces of content: a YouTube
//! long-form, three Shorts, a TikTok set, a Patreon-exclusive cut, an
//! audio-only podcast feed entry, a blog draft. This plugin builds
//! the draft set automatically from a source recording + the artefacts
//! upstream plugins already produced (Clipper highlights, Chapters
//! markers, Crunchr transcripts, Thumbnails, Brandsafe verdicts).
//!
//! The drafter is **pure** — no IO, no API calls, no publishing. It
//! shapes data for the SPA queue; the user (or a future real
//! publisher backend) takes the next step. The `Format`, `PublishDraft`
//! shapes carry enough information that a downstream YouTube /
//! TikTok / Patreon API integration plugs in by reading the same
//! draft rows.
//!
//! Six default destinations:
//!
//!   * `YouTubeLong`   — 16:9, full duration, full description
//!   * `YouTubeShort`  — 9:16, ≤60s, top-3 highlights
//!   * `TikTok`        — 9:16, ≤60s, top-3 highlights
//!   * `Patreon`       — original ratio, full duration, exclusive copy
//!   * `Podcast`       — audio-only, original duration
//!   * `Blog`          — markdown writeup (transcript-driven)

use serde::{Deserialize, Serialize};

pub mod store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    YouTubeLong,
    YouTubeShort,
    TikTok,
    Patreon,
    Podcast,
    Blog,
}

impl Format {
    pub fn id(&self) -> &'static str {
        match self {
            Format::YouTubeLong => "youtube_long",
            Format::YouTubeShort => "youtube_short",
            Format::TikTok => "tiktok",
            Format::Patreon => "patreon",
            Format::Podcast => "podcast",
            Format::Blog => "blog",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Format::YouTubeLong => "YouTube (long)",
            Format::YouTubeShort => "YouTube Shorts",
            Format::TikTok => "TikTok",
            Format::Patreon => "Patreon exclusive",
            Format::Podcast => "Podcast (audio)",
            Format::Blog => "Blog draft",
        }
    }
    pub fn aspect(&self) -> &'static str {
        match self {
            Format::YouTubeLong | Format::Patreon => "16:9",
            Format::YouTubeShort | Format::TikTok => "9:16",
            Format::Podcast => "audio",
            Format::Blog => "text",
        }
    }
    pub fn duration_cap(&self) -> Option<f32> {
        match self {
            Format::YouTubeShort | Format::TikTok => Some(60.0),
            _ => None,
        }
    }
}

/// All six default destinations.
pub const DEFAULT_FORMATS: &[Format] = &[
    Format::YouTubeLong,
    Format::YouTubeShort,
    Format::TikTok,
    Format::Patreon,
    Format::Podcast,
    Format::Blog,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecording {
    pub recording_id: String,
    pub title: String,
    pub channel_name: String,
    pub source_path: String,
    pub duration_sec: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DraftInputs {
    /// Top words from Insights — used for hashtag generation.
    #[serde(default)]
    pub top_words: Vec<String>,
    /// Topic list — secondary hashtag source.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Clip start-times the user already cut (from Clipper). When
    /// non-empty, Shorts / TikTok drafts target the top-3.
    #[serde(default)]
    pub clip_starts: Vec<f32>,
    /// Chapter list as `HH:MM:SS  Title` lines — flowed into the YT
    /// description and the blog draft.
    #[serde(default)]
    pub chapters_block: String,
    /// Summary blurb from Crunchr — leads the description.
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishDraft {
    pub format: Format,
    pub title: String,
    pub description: String,
    pub hashtags: Vec<String>,
    pub source_path: String,
    pub duration_sec: f32,
    /// Per-clip start_sec offsets when the format slices the source
    /// (Shorts / TikTok). Empty for full-length destinations.
    pub clip_starts: Vec<f32>,
    pub aspect: String,
    pub status: DraftStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    Queued,
    Approved,
    Published,
    Dropped,
}

/// Build a draft set for every default destination.
pub fn generate_drafts(rec: &SourceRecording, inputs: &DraftInputs) -> Vec<PublishDraft> {
    DEFAULT_FORMATS
        .iter()
        .map(|f| make_draft(*f, rec, inputs))
        .collect()
}

/// Build a single draft for one format. Caller can call this with a
/// custom format set (e.g. only YouTubeShort + TikTok) to skip the
/// destinations they don't ship to.
pub fn make_draft(format: Format, rec: &SourceRecording, inputs: &DraftInputs) -> PublishDraft {
    let title = title_for(format, rec);
    let description = description_for(format, rec, inputs);
    let hashtags = hashtags_for(format, &inputs.top_words, &inputs.topics);
    let duration_sec = match format.duration_cap() {
        Some(cap) => cap.min(rec.duration_sec),
        None => rec.duration_sec,
    };
    let clip_starts = match format {
        Format::YouTubeShort | Format::TikTok => {
            inputs.clip_starts.iter().take(3).copied().collect()
        }
        _ => Vec::new(),
    };
    PublishDraft {
        format,
        title,
        description,
        hashtags,
        source_path: rec.source_path.clone(),
        duration_sec,
        clip_starts,
        aspect: format.aspect().to_string(),
        status: DraftStatus::Queued,
    }
}

/// Generate a format-appropriate title. Long-form uses the original;
/// vertical formats prepend a #Shorts / #TikTok prefix in
/// `description` but stay short on the title itself.
fn title_for(format: Format, rec: &SourceRecording) -> String {
    let base = rec.title.trim();
    match format {
        Format::YouTubeLong | Format::Patreon | Format::Podcast => base.to_string(),
        Format::Blog => format!("Show notes: {base}"),
        Format::YouTubeShort | Format::TikTok => {
            // Clip the title to 70 chars — both platforms cut around there.
            let mut t = base.to_string();
            if t.len() > 70 {
                t.truncate(67);
                t.push_str("...");
            }
            t
        }
    }
}

/// Format-specific description body.
fn description_for(format: Format, rec: &SourceRecording, inputs: &DraftInputs) -> String {
    let mut out = String::new();
    if !inputs.summary.is_empty() {
        out.push_str(&inputs.summary);
        out.push_str("\n\n");
    }
    match format {
        Format::YouTubeLong => {
            if !inputs.chapters_block.is_empty() {
                out.push_str("Chapters:\n");
                out.push_str(&inputs.chapters_block);
                out.push_str("\n\n");
            }
            out.push_str(&format!("Recorded live with {}.\n", rec.channel_name));
        }
        Format::Patreon => {
            out.push_str("Patron-exclusive cut.\n");
            if !inputs.chapters_block.is_empty() {
                out.push_str("Chapters:\n");
                out.push_str(&inputs.chapters_block);
                out.push('\n');
            }
        }
        Format::YouTubeShort | Format::TikTok => {
            out.push_str("Top moments from the full stream — link in profile.\n");
            if !inputs.clip_starts.is_empty() {
                out.push_str("Highlights at: ");
                out.push_str(
                    &inputs
                        .clip_starts
                        .iter()
                        .take(3)
                        .map(|t| format_clock(*t))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                out.push('\n');
            }
        }
        Format::Podcast => {
            out.push_str(&format!("Audio cut of the {} stream.\n", rec.channel_name));
            if !inputs.chapters_block.is_empty() {
                out.push_str("Chapters:\n");
                out.push_str(&inputs.chapters_block);
                out.push('\n');
            }
        }
        Format::Blog => {
            out.push_str(&format!("# {}\n\n", rec.title));
            if !inputs.summary.is_empty() {
                out.push_str(&inputs.summary);
                out.push_str("\n\n");
            }
            if !inputs.chapters_block.is_empty() {
                out.push_str("## Sections\n\n");
                out.push_str(&inputs.chapters_block);
                out.push_str("\n\n");
            }
            out.push_str("---\nPosted by Chorosyne.\n");
        }
    }
    out.trim_end().to_string()
}

fn format_clock(sec: f32) -> String {
    let s = sec.max(0.0) as u64;
    let h = s / 3600;
    let m = (s / 60) % 60;
    let r = s % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{r:02}")
    } else {
        format!("{m:02}:{r:02}")
    }
}

/// Generate format-specific hashtags. Stopwords stripped; per-format
/// platform tag prepended; deduped while preserving order.
pub fn hashtags_for(format: Format, top_words: &[String], topics: &[String]) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    let platform_tag = match format {
        Format::YouTubeShort => Some("Shorts"),
        Format::TikTok => Some("TikTok"),
        _ => None,
    };
    if let Some(p) = platform_tag {
        tags.push(format!("#{p}"));
    }
    let mut seen = std::collections::HashSet::new();
    let push_tag = |s: &str, tags: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        let cleaned: String = s
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        if cleaned.len() < 3 {
            return;
        }
        let lower = cleaned.to_lowercase();
        if seen.insert(lower) {
            tags.push(format!("#{}", title_case(&cleaned)));
        }
    };
    let cap = match format {
        Format::YouTubeShort | Format::TikTok => 10,
        Format::YouTubeLong => 8,
        _ => 6,
    };
    for t in topics.iter() {
        if tags.len() >= cap {
            break;
        }
        push_tag(t, &mut tags, &mut seen);
    }
    for w in top_words.iter() {
        if tags.len() >= cap {
            break;
        }
        push_tag(w, &mut tags, &mut seen);
    }
    tags
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(duration: f32) -> SourceRecording {
        SourceRecording {
            recording_id: "r1".into(),
            title: "Diablo Speedrun Stream — World Record Attempt".into(),
            channel_name: "ChannelX".into(),
            source_path: "/tmp/x.mkv".into(),
            duration_sec: duration,
        }
    }

    fn inputs() -> DraftInputs {
        DraftInputs {
            top_words: vec!["diablo".into(), "speedrun".into(), "hardcore".into()],
            topics: vec!["Diablo".into(), "World Record".into()],
            clip_starts: vec![120.0, 800.0, 1500.0, 2400.0],
            chapters_block: "00:00 Intro\n01:30 Boss Fight".into(),
            summary: "World record attempt across all bosses.".into(),
        }
    }

    #[test]
    fn generate_drafts_covers_every_default_format() {
        let drafts = generate_drafts(&rec(7200.0), &inputs());
        assert_eq!(drafts.len(), DEFAULT_FORMATS.len());
        let ids: Vec<&str> = drafts.iter().map(|d| d.format.id()).collect();
        for f in DEFAULT_FORMATS {
            assert!(ids.contains(&f.id()));
        }
    }

    #[test]
    fn shorts_and_tiktok_cap_duration_to_60s() {
        let drafts = generate_drafts(&rec(7200.0), &inputs());
        let short = drafts.iter().find(|d| d.format == Format::YouTubeShort).unwrap();
        let tt = drafts.iter().find(|d| d.format == Format::TikTok).unwrap();
        assert_eq!(short.duration_sec, 60.0);
        assert_eq!(tt.duration_sec, 60.0);
    }

    #[test]
    fn shorts_keeps_top_3_clip_starts() {
        let drafts = generate_drafts(&rec(7200.0), &inputs());
        let short = drafts.iter().find(|d| d.format == Format::YouTubeShort).unwrap();
        assert_eq!(short.clip_starts.len(), 3);
        assert_eq!(short.clip_starts, vec![120.0, 800.0, 1500.0]);
    }

    #[test]
    fn long_form_keeps_full_duration() {
        let drafts = generate_drafts(&rec(7200.0), &inputs());
        let long = drafts.iter().find(|d| d.format == Format::YouTubeLong).unwrap();
        assert_eq!(long.duration_sec, 7200.0);
        assert!(long.clip_starts.is_empty());
    }

    #[test]
    fn shorts_title_truncates_at_70_chars() {
        let mut r = rec(60.0);
        r.title = "x".repeat(120);
        let drafts = generate_drafts(&r, &inputs());
        let short = drafts.iter().find(|d| d.format == Format::YouTubeShort).unwrap();
        assert!(short.title.len() <= 70);
        assert!(short.title.ends_with("..."));
    }

    #[test]
    fn blog_title_prefixes_show_notes() {
        let drafts = generate_drafts(&rec(60.0), &inputs());
        let blog = drafts.iter().find(|d| d.format == Format::Blog).unwrap();
        assert!(blog.title.starts_with("Show notes:"));
    }

    #[test]
    fn hashtags_for_shorts_includes_platform_tag() {
        let tags = hashtags_for(Format::YouTubeShort, &["alpha".into()], &["beta".into()]);
        assert!(tags.contains(&"#Shorts".to_string()));
    }

    #[test]
    fn hashtags_dedup_topic_word_overlap_case_insensitive() {
        let tags = hashtags_for(Format::YouTubeLong, &["Diablo".into()], &["diablo".into()]);
        let lowered: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
        let count = lowered.iter().filter(|t| *t == "#diablo").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn hashtags_drop_too_short_terms() {
        let tags = hashtags_for(Format::YouTubeLong, &["ab".into(), "ok".into()], &[]);
        assert!(tags.is_empty(), "got {tags:?}");
    }

    #[test]
    fn hashtags_strip_non_alphanumeric() {
        let tags = hashtags_for(Format::YouTubeLong, &["World Record!".into()], &[]);
        assert!(tags.iter().any(|t| t == "#WorldRecord"));
    }

    #[test]
    fn description_for_youtube_long_includes_chapters_block() {
        let drafts = generate_drafts(&rec(7200.0), &inputs());
        let long = drafts.iter().find(|d| d.format == Format::YouTubeLong).unwrap();
        assert!(long.description.contains("Chapters:"));
        assert!(long.description.contains("00:00 Intro"));
    }

    #[test]
    fn description_for_blog_uses_markdown() {
        let drafts = generate_drafts(&rec(7200.0), &inputs());
        let blog = drafts.iter().find(|d| d.format == Format::Blog).unwrap();
        assert!(blog.description.contains("# "));
        assert!(blog.description.contains("## Sections"));
    }
}
