//! strivo-casebook — post-stream report composer.
//!
//! The end-of-stream notebook every solo streamer wishes existed: one
//! markdown briefing that pulls together every upstream plugin's
//! findings into a single document.
//!
//! Inputs (any of these can be empty — Casebook degrades gracefully):
//!   * Crunchr   — summary, transcript metadata
//!   * Chapters  — chapter list
//!   * Clipper   — top highlight candidates
//!   * Viewguard — viewbot verdict + score
//!   * Brandsafe — content-safety verdicts
//!   * Insights  — top words, topic chips
//!
//! Output: [`CasebookReport`] (typed surface for the SPA) +
//! [`to_markdown`] (the file the streamer downloads).
//!
//! Pure data + string formatting; runs without IO.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CasebookInputs {
    pub recording_id: String,
    pub title: String,
    pub channel_name: String,
    pub started_at: Option<String>,
    pub duration_sec: f32,

    /// Summary from Crunchr.
    #[serde(default)]
    pub summary: String,
    /// Topics (Crunchr / Insights).
    #[serde(default)]
    pub topics: Vec<String>,
    /// Top-N words.
    #[serde(default)]
    pub top_words: Vec<WordCount>,
    /// Chapter markers (start_sec + title).
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    /// Highlight candidates (start_sec + score 0..1).
    #[serde(default)]
    pub highlights: Vec<Highlight>,
    /// Viewbot verdict score in [0, 1] (Viewguard).
    #[serde(default)]
    pub viewbot_score: Option<f32>,
    /// Brand-safety verdict count by severity.
    #[serde(default)]
    pub brandsafe_counts: BrandsafeCounts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrandsafeCounts {
    #[serde(default)]
    pub critical: u32,
    #[serde(default)]
    pub high: u32,
    #[serde(default)]
    pub medium: u32,
    #[serde(default)]
    pub low: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordCount {
    pub word: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub start_sec: f32,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Highlight {
    pub time_sec: f32,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasebookReport {
    pub recording_id: String,
    pub title: String,
    pub channel_name: String,
    pub started_at: Option<String>,
    pub duration_sec: f32,
    pub sections: Vec<Section>,
    /// Suggested title rewrites — heuristic from top topics.
    pub suggested_titles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub heading: String,
    pub body: String,
}

/// Compose a [`CasebookReport`] from inputs. Empty inputs produce
/// empty sections rather than missing fields, so the SPA can render
/// a stable shape.
pub fn compose_report(inputs: &CasebookInputs) -> CasebookReport {
    let mut sections: Vec<Section> = Vec::new();

    // Overview — always present.
    let overview = format!(
        "**Channel:** {}\n**Duration:** {}\n**Recorded:** {}\n",
        inputs.channel_name,
        format_clock(inputs.duration_sec),
        inputs.started_at.as_deref().unwrap_or("—"),
    );
    sections.push(Section { heading: "Overview".into(), body: overview });

    if !inputs.summary.is_empty() {
        sections.push(Section {
            heading: "Summary".into(),
            body: inputs.summary.trim().to_string(),
        });
    }

    if !inputs.topics.is_empty() {
        let body = inputs
            .topics
            .iter()
            .take(20)
            .map(|t| format!("- {t}"))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(Section { heading: "Topics".into(), body });
    }

    if !inputs.chapters.is_empty() {
        let body = inputs
            .chapters
            .iter()
            .map(|c| format!("- {} — {}", format_clock(c.start_sec), c.title))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(Section { heading: "Chapters".into(), body });
    }

    if !inputs.highlights.is_empty() {
        let body = inputs
            .highlights
            .iter()
            .take(10)
            .map(|h| {
                format!(
                    "- {} — score {:.0}%",
                    format_clock(h.time_sec),
                    (h.score.clamp(0.0, 1.0) * 100.0).round()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(Section { heading: "Top clip candidates".into(), body });
    }

    if let Some(score) = inputs.viewbot_score {
        let pct = (score.clamp(0.0, 1.0) * 100.0).round();
        let verdict = if pct >= 70.0 {
            "🚨 High viewbot risk — review Viewguard samples"
        } else if pct >= 35.0 {
            "⚠ Moderate viewbot signal"
        } else {
            "✓ Viewbot signal low"
        };
        sections.push(Section {
            heading: "Viewbot integrity".into(),
            body: format!("**Score:** {pct}% — {verdict}"),
        });
    }

    let bs = &inputs.brandsafe_counts;
    if bs.critical + bs.high + bs.medium + bs.low > 0 {
        let body = format!(
            "- **Critical:** {}\n- **High:** {}\n- **Medium:** {}\n- **Low:** {}",
            bs.critical, bs.high, bs.medium, bs.low
        );
        sections.push(Section { heading: "Brand-safety verdicts".into(), body });
    } else {
        sections.push(Section {
            heading: "Brand-safety verdicts".into(),
            body: "✓ No risks flagged.".into(),
        });
    }

    if !inputs.top_words.is_empty() {
        let body = inputs
            .top_words
            .iter()
            .take(15)
            .map(|w| format!("- `{}` × {}", w.word, w.count))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(Section { heading: "Top words".into(), body });
    }

    let suggested_titles = suggest_titles(inputs);

    CasebookReport {
        recording_id: inputs.recording_id.clone(),
        title: inputs.title.clone(),
        channel_name: inputs.channel_name.clone(),
        started_at: inputs.started_at.clone(),
        duration_sec: inputs.duration_sec,
        sections,
        suggested_titles,
    }
}

/// Heuristic title suggestions. Pulls top-3 topics, casefolds, joins
/// with separators that read well on social platforms.
pub fn suggest_titles(inputs: &CasebookInputs) -> Vec<String> {
    let mut topics_or_words: Vec<String> = inputs
        .topics
        .iter()
        .take(3)
        .cloned()
        .collect();
    if topics_or_words.len() < 3 {
        for w in inputs.top_words.iter().take(3 - topics_or_words.len()) {
            topics_or_words.push(w.word.clone());
        }
    }
    let mut titled: Vec<String> = topics_or_words.iter().map(|t| title_case(t)).collect();
    titled.sort();
    titled.dedup();
    if titled.is_empty() {
        return Vec::new();
    }
    let joined = titled.join(" + ");
    let mut out = vec![
        format!("{} — {}", joined, inputs.channel_name),
        format!("{} (full stream VOD)", joined),
    ];
    if !inputs.highlights.is_empty() {
        out.push(format!("Top {} moments from {}", inputs.highlights.len(), joined));
    }
    out
}

fn title_case(s: &str) -> String {
    let trimmed = s.trim();
    let mut chars = trimmed.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
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

/// Render a [`CasebookReport`] as a markdown document. Title becomes
/// the H1; each section becomes an H2 with its body verbatim;
/// suggested titles trail at the end.
pub fn to_markdown(report: &CasebookReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Casebook · {}\n\n", report.title));
    for s in &report.sections {
        out.push_str(&format!("## {}\n\n{}\n\n", s.heading, s.body));
    }
    if !report.suggested_titles.is_empty() {
        out.push_str("## Suggested titles\n\n");
        for t in &report.suggested_titles {
            out.push_str(&format!("- {t}\n"));
        }
        out.push('\n');
    }
    out.push_str("---\n_Generated by StriVo Casebook._\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_inputs() -> CasebookInputs {
        CasebookInputs {
            recording_id: "r1".into(),
            title: "Diablo speedrun".into(),
            channel_name: "ChannelX".into(),
            started_at: Some("2026-05-28T12:00:00Z".into()),
            duration_sec: 7200.0,
            summary: "World-record attempt across all bosses.".into(),
            topics: vec!["Diablo".into(), "Speedrun".into(), "Hardcore".into()],
            top_words: vec![
                WordCount { word: "diablo".into(), count: 92 },
                WordCount { word: "boss".into(), count: 41 },
            ],
            chapters: vec![
                Chapter { start_sec: 0.0, title: "Intro".into() },
                Chapter { start_sec: 1800.0, title: "Diablo".into() },
            ],
            highlights: vec![
                Highlight { time_sec: 600.0, score: 0.92 },
                Highlight { time_sec: 4500.0, score: 0.74 },
            ],
            viewbot_score: Some(0.15),
            brandsafe_counts: BrandsafeCounts { critical: 0, high: 0, medium: 1, low: 2 },
        }
    }

    #[test]
    fn compose_emits_overview_always() {
        let r = compose_report(&CasebookInputs::default());
        assert!(r.sections.iter().any(|s| s.heading == "Overview"));
    }

    #[test]
    fn compose_includes_all_sections_when_inputs_populated() {
        let r = compose_report(&base_inputs());
        let headings: Vec<&str> = r.sections.iter().map(|s| s.heading.as_str()).collect();
        for needed in [
            "Overview", "Summary", "Topics", "Chapters", "Top clip candidates",
            "Viewbot integrity", "Brand-safety verdicts", "Top words",
        ] {
            assert!(headings.contains(&needed), "missing section: {needed} — got {headings:?}");
        }
    }

    #[test]
    fn compose_handles_empty_inputs_without_panicking() {
        let r = compose_report(&CasebookInputs::default());
        // Overview + brand-safety (no-risk) survive on empty input.
        assert!(r.sections.len() >= 2);
    }

    #[test]
    fn viewbot_high_score_emits_alarm() {
        let mut inp = CasebookInputs::default();
        inp.viewbot_score = Some(0.85);
        let r = compose_report(&inp);
        let section = r.sections.iter().find(|s| s.heading == "Viewbot integrity").unwrap();
        assert!(section.body.contains("High viewbot risk"));
    }

    #[test]
    fn viewbot_low_score_emits_clear() {
        let mut inp = CasebookInputs::default();
        inp.viewbot_score = Some(0.05);
        let r = compose_report(&inp);
        let section = r.sections.iter().find(|s| s.heading == "Viewbot integrity").unwrap();
        assert!(section.body.contains("Viewbot signal low"));
    }

    #[test]
    fn brandsafe_no_counts_emits_clean_marker() {
        let r = compose_report(&CasebookInputs::default());
        let section = r.sections.iter().find(|s| s.heading == "Brand-safety verdicts").unwrap();
        assert!(section.body.contains("No risks flagged"));
    }

    #[test]
    fn suggested_titles_pull_from_topics_first() {
        let r = compose_report(&base_inputs());
        // Topics ["Diablo", "Speedrun", "Hardcore"] → titles include them.
        assert!(r.suggested_titles.iter().any(|t| t.contains("Diablo")));
        assert!(r.suggested_titles.iter().any(|t| t.contains("Speedrun")));
    }

    #[test]
    fn suggested_titles_fall_back_to_top_words() {
        let mut inp = CasebookInputs::default();
        inp.top_words = vec![
            WordCount { word: "alpha".into(), count: 5 },
            WordCount { word: "beta".into(), count: 4 },
            WordCount { word: "gamma".into(), count: 3 },
        ];
        let titles = suggest_titles(&inp);
        assert!(!titles.is_empty());
        let joined = titles.join(" ");
        assert!(joined.contains("Alpha"));
        assert!(joined.contains("Beta"));
        assert!(joined.contains("Gamma"));
    }

    #[test]
    fn markdown_renders_h1_h2_and_outro() {
        let r = compose_report(&base_inputs());
        let md = to_markdown(&r);
        assert!(md.starts_with("# Casebook · Diablo speedrun"));
        assert!(md.contains("## Overview"));
        assert!(md.contains("## Topics"));
        assert!(md.contains("## Suggested titles"));
        assert!(md.contains("_Generated by StriVo Casebook._"));
    }

    #[test]
    fn markdown_round_trip_preserves_chapter_lines() {
        let r = compose_report(&base_inputs());
        let md = to_markdown(&r);
        assert!(md.contains("00:00 — Intro"));
        assert!(md.contains("30:00 — Diablo"));
    }

    #[test]
    fn highlights_capped_to_ten_lines() {
        let mut inp = CasebookInputs::default();
        inp.highlights = (0..50)
            .map(|i| Highlight { time_sec: i as f32 * 10.0, score: 0.5 })
            .collect();
        let r = compose_report(&inp);
        let section = r.sections.iter().find(|s| s.heading == "Top clip candidates").unwrap();
        let line_count = section.body.lines().count();
        assert!(line_count <= 10, "got {line_count}");
    }
}
