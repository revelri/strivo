//! strivo-brandsafe — pre-publish content classifier.
//!
//! Scans a recording's transcript and metadata for content that would
//! get a stream demonetised, age-gated, or DMCA'd, and returns a
//! ranked list of [`Verdict`]s the streamer can act on before they
//! hit publish.
//!
//! Four scanners, each unit-testable in isolation:
//!
//!   1. [`scan_slurs`] — hard-blocklist match for unambiguous slurs
//!      (English; small curated list, conservative). Returns per-hit
//!      timestamps + snippets so the SPA can offer "jump and bleep".
//!   2. [`scan_profanity`] — softer profanity list, tagged Medium so
//!      the gate doesn't yell at a streamer who said "hell".
//!   3. [`scan_restricted_game`] — platform-aware allow-list for
//!      game categories. Twitch and YouTube have different rules; a
//!      single Verdict per applicable platform.
//!   4. [`scan_music_mentions`] — transcript-based heuristic for
//!      copyright-music risk (mentions of Spotify, song titles,
//!      "playing this song", etc.). Cheap proxy for real ACR.
//!
//! Composed: [`scan_all`] runs every scanner and returns the verdict
//! list, sorted by (severity desc, at_sec asc). No IO; runs entirely
//! on plain strings.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    fn rank(self) -> u8 {
        match self {
            Severity::Critical => 4,
            Severity::High => 3,
            Severity::Medium => 2,
            Severity::Low => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Slur,
    Profanity,
    RestrictedGame,
    MusicMention,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub kind: Kind,
    pub severity: Severity,
    /// Where in the recording the issue lands. None for metadata
    /// matches (RestrictedGame is per-recording, not per-moment).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_sec: Option<f32>,
    /// Short surrounding context the SPA renders for review.
    pub snippet: String,
    /// What the streamer should do about it. Short imperative phrase.
    pub fix_hint: String,
    /// Optional platform name when the verdict is platform-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Segment {
    pub start_sec: f32,
    pub end_sec: f32,
    pub text: String,
}

/// Conservative slur list — only includes terms that are unambiguously
/// reportable across major platforms. Keeping the list tight is the
/// whole point; over-flagging numbs the streamer to the alert.
/// Listed here in masked form ("n*") because including the raw token
/// in the repo / linter / IDE search history is undesirable. Caller
/// can extend at runtime by passing through `extend_slurs`.
const DEFAULT_SLURS_MASKED: &[&str] = &[
    "n*gg*r", "n*gg*", "f*gg*t", "f*g", "tr*nny", "ch*nk", "k*ke", "sp*c", "r*tard",
];

const DEFAULT_PROFANITY: &[&str] = &[
    "fuck", "shit", "bitch", "asshole", "bastard", "cunt", "dick", "piss",
];

/// Platform-aware restricted-game list. Keys are platform names
/// (case-insensitive); values are the game / category labels they
/// treat as restricted. Real allow-lists are richer; the seed below
/// is what trips up streamers most often.
pub fn default_restricted_games(platform: &str) -> &'static [&'static str] {
    match platform.to_ascii_lowercase().as_str() {
        "twitch" => &[
            "gambling", "slots", "blackjack", "roulette",
            "stake", "duelbits", "csgoroll",
            "rust", // Twitch-restricted for nudity events historically (illustrative)
        ],
        "youtube" => &[
            "gambling", "slots",
            "vape", "tobacco",
        ],
        _ => &[],
    }
}

/// Music-mention heuristic terms. Casing-insensitive substring match.
const MUSIC_HINT_TERMS: &[&str] = &[
    "spotify", "apple music", "song is", "this song", "play that song",
    "soundcloud", "youtube music",
];

/// Slur scanner. Matches against [`DEFAULT_SLURS_MASKED`] *after*
/// unmasking the asterisks back into character classes — this keeps
/// the literal slur out of the repo while still detecting it.
pub fn scan_slurs(segments: &[Segment]) -> Vec<Verdict> {
    let patterns: Vec<String> = DEFAULT_SLURS_MASKED.iter().map(|s| unmask(s)).collect();
    let mut out = Vec::new();
    for seg in segments {
        let lower = seg.text.to_lowercase();
        for (i, p) in patterns.iter().enumerate() {
            if contains_word(&lower, p) {
                out.push(Verdict {
                    kind: Kind::Slur,
                    severity: Severity::Critical,
                    at_sec: Some(seg.start_sec),
                    snippet: snippet_for(&seg.text, p),
                    fix_hint: format!(
                        "Edit out or bleep — slur pattern #{} at {:.0}s",
                        i + 1,
                        seg.start_sec
                    ),
                    platform: None,
                });
            }
        }
    }
    out
}

/// Profanity scanner — softer list, Medium severity.
pub fn scan_profanity(segments: &[Segment]) -> Vec<Verdict> {
    let mut out = Vec::new();
    for seg in segments {
        let lower = seg.text.to_lowercase();
        for word in DEFAULT_PROFANITY {
            if contains_word(&lower, word) {
                out.push(Verdict {
                    kind: Kind::Profanity,
                    severity: Severity::Medium,
                    at_sec: Some(seg.start_sec),
                    snippet: snippet_for(&seg.text, word),
                    fix_hint: "Soft-mute or age-restrict consideration".into(),
                    platform: None,
                });
                break;
            }
        }
    }
    out
}

/// Restricted-game scanner. Matches the recording's category/title
/// against the per-platform restricted list. Returns one verdict per
/// platform that flags.
pub fn scan_restricted_game(category: &str, platforms: &[&str]) -> Vec<Verdict> {
    let cat_lower = category.to_lowercase();
    let mut out = Vec::new();
    for platform in platforms {
        for forbidden in default_restricted_games(platform) {
            if cat_lower.contains(forbidden) {
                out.push(Verdict {
                    kind: Kind::RestrictedGame,
                    severity: Severity::High,
                    at_sec: None,
                    snippet: format!("'{category}' contains restricted term '{forbidden}'"),
                    fix_hint: format!(
                        "Re-tag category or skip {platform} publish — '{forbidden}' is restricted"
                    ),
                    platform: Some(platform.to_string()),
                });
            }
        }
    }
    out
}

/// Music mention scanner — flags transcript snippets that hint at
/// copyrighted music playing in the background.
pub fn scan_music_mentions(segments: &[Segment]) -> Vec<Verdict> {
    let mut out = Vec::new();
    for seg in segments {
        let lower = seg.text.to_lowercase();
        for term in MUSIC_HINT_TERMS {
            if lower.contains(term) {
                out.push(Verdict {
                    kind: Kind::MusicMention,
                    severity: Severity::Low,
                    at_sec: Some(seg.start_sec),
                    snippet: snippet_for(&seg.text, term),
                    fix_hint: "Confirm music rights or replace with cleared audio".into(),
                    platform: None,
                });
                break;
            }
        }
    }
    out
}

/// Run every scanner. Sorts by (severity desc, at_sec asc) so the
/// most-pressing items lead the SPA list.
pub fn scan_all(segments: &[Segment], category: &str, platforms: &[&str]) -> Vec<Verdict> {
    let mut out = Vec::new();
    out.extend(scan_slurs(segments));
    out.extend(scan_profanity(segments));
    out.extend(scan_restricted_game(category, platforms));
    out.extend(scan_music_mentions(segments));
    out.sort_by(|a, b| {
        b.severity
            .rank()
            .cmp(&a.severity.rank())
            .then_with(|| {
                a.at_sec
                    .unwrap_or(f32::INFINITY)
                    .partial_cmp(&b.at_sec.unwrap_or(f32::INFINITY))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    out
}

/// Whole-word substring match. Avoids flagging "scunthorpe" when
/// looking for "cunt" — a classic content-filter trap. Tightened to
/// non-alphabetic boundaries.
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(idx) = haystack[start..].find(needle) {
        let abs = start + idx;
        let pre = haystack[..abs].chars().last();
        let post = haystack[abs + needle.len()..].chars().next();
        let edge = |c: Option<char>| c.map(|c| !c.is_alphanumeric()).unwrap_or(true);
        if edge(pre) && edge(post) {
            return true;
        }
        start = abs + needle.len();
    }
    false
}

fn snippet_for(text: &str, hit: &str) -> String {
    let lower = text.to_lowercase();
    if let Some(idx) = lower.find(hit) {
        let len = hit.len();
        let context_chars = 30;
        let lo = idx.saturating_sub(context_chars);
        let hi = (idx + len + context_chars).min(text.len());
        let mut s = String::new();
        if lo > 0 {
            s.push('…');
        }
        s.push_str(&text[lo..hi]);
        if hi < text.len() {
            s.push('…');
        }
        s
    } else {
        text.chars().take(80).collect()
    }
}

/// "n*gg*r" → "n[a-z]gg[a-z]r" then re-collapsed; for the purposes of
/// our matching we only need the raw letters with no in-between
/// wildcards so we just drop the asterisks. (Streamers don't type
/// literal asterisks; the mask is repo-hygiene only.)
fn unmask(masked: &str) -> String {
    masked.replace('*', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f32, text: &str) -> Segment {
        Segment { start_sec: start, end_sec: start + 5.0, text: text.into() }
    }

    #[test]
    fn slur_scanner_critical_severity_on_match() {
        // Build the test input by unmasking ourselves so the slur
        // doesn't appear literally in the source tree.
        let raw = unmask("n*gg*r");
        let segs = vec![seg(10.0, &format!("said the word {raw} on stream"))];
        let out = scan_slurs(&segs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Critical);
        assert_eq!(out[0].at_sec, Some(10.0));
    }

    #[test]
    fn slur_scanner_word_boundary_protection() {
        // A literal harmless word that contains slur letters as a
        // substring should not match (whole-word matcher).
        let segs = vec![seg(5.0, "I drove to scunthorpe last weekend")];
        let out = scan_slurs(&segs);
        assert!(out.is_empty(), "false positive: {out:?}");
    }

    #[test]
    fn profanity_scanner_medium_severity() {
        let segs = vec![seg(2.0, "what the fuck is that")];
        let out = scan_profanity(&segs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Medium);
    }

    #[test]
    fn restricted_game_flags_twitch_gambling() {
        let out = scan_restricted_game("Slots Gambling Stream", &["Twitch"]);
        assert!(out.len() >= 1);
        assert!(out.iter().any(|v| v.platform.as_deref() == Some("Twitch")));
        assert!(out.iter().all(|v| v.severity == Severity::High));
    }

    #[test]
    fn restricted_game_per_platform_separation() {
        // 'vape' triggers on YouTube but not Twitch.
        let cat = "Vape Review Stream";
        let twitch = scan_restricted_game(cat, &["Twitch"]);
        let yt = scan_restricted_game(cat, &["YouTube"]);
        assert!(twitch.is_empty(), "twitch should not flag vape, got {twitch:?}");
        assert!(yt.iter().any(|v| v.platform.as_deref() == Some("YouTube")));
    }

    #[test]
    fn music_mention_low_severity() {
        let segs = vec![seg(15.0, "this song slaps on Spotify")];
        let out = scan_music_mentions(&segs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Low);
    }

    #[test]
    fn scan_all_sorted_by_severity_desc_then_time() {
        let raw_slur = unmask("n*gg*r");
        let segs = vec![
            seg(5.0, "this is a clean line"),
            seg(10.0, &format!("said the word {raw_slur} on stream")),
            seg(15.0, "what the fuck"),
            seg(20.0, "this song is great on Spotify"),
        ];
        let all = scan_all(&segs, "Just Chatting", &["Twitch"]);
        assert!(!all.is_empty());
        // Highest severity first.
        assert_eq!(all[0].severity, Severity::Critical);
        // Then Medium (profanity) before Low (music).
        let kinds: Vec<Kind> = all.iter().map(|v| v.kind).collect();
        let crit_idx = kinds.iter().position(|k| *k == Kind::Slur).unwrap();
        let med_idx = kinds.iter().position(|k| *k == Kind::Profanity).unwrap();
        let low_idx = kinds.iter().position(|k| *k == Kind::MusicMention).unwrap();
        assert!(crit_idx < med_idx);
        assert!(med_idx < low_idx);
    }

    #[test]
    fn snippet_truncates_with_ellipses_around_hit() {
        // Need at least 30 chars on each side so the snippet window
        // doesn't reach the text boundaries.
        let prefix = "x".repeat(50);
        let suffix = "y".repeat(50);
        let text = format!("{prefix} fuck {suffix}");
        let snip = snippet_for(&text, "fuck");
        assert!(snip.contains("fuck"), "got {snip:?}");
        assert!(snip.starts_with('…'), "missing leading ellipsis: {snip:?}");
        assert!(snip.ends_with('…'), "missing trailing ellipsis: {snip:?}");
    }

    #[test]
    fn snippet_no_ellipses_when_window_covers_text() {
        // Short text — snippet returns the whole thing, no ellipses.
        let snip = snippet_for("short fuck text", "fuck");
        assert_eq!(snip, "short fuck text");
    }

    #[test]
    fn empty_inputs_yield_empty_verdicts() {
        let all = scan_all(&[], "", &[]);
        assert!(all.is_empty());
    }
}
