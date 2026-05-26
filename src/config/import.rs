//! Bulk-import auto-record entries from OBS / Streamlink user configs
//! (M5.7).
//!
//! The two formats StriVo bothers with today:
//!
//! - **OBS scene collection JSON** — exported from OBS Studio under
//!   *Scene Collection → Export*. We walk the `sources` array,
//!   matching `id: "ffmpeg_source"` / `id: "rtmp_source"` /
//!   `id: "browser_source"` rows and extracting any twitch.tv or
//!   youtube.com URL from their `settings.input` / `settings.url`.
//!
//! - **Streamlink listing** — either a `~/.config/streamlink/config`
//!   file or a plain `streams.txt`-style file. We grep each non-comment
//!   line for a known platform URL.
//!
//! Both parsers return [`Candidate`] entries that the caller previews
//! before persisting (no auto-commit without an explicit `--apply`
//! flag, so a stale or shared file doesn't silently rewrite the
//! user's `[[auto_record_channels]]`).

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::AutoRecordEntry;

/// A single discovered channel — pre-`AutoRecordEntry` so the caller
/// can dedupe against the existing config before promoting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub platform: String,
    /// Channel slug or numeric ID, whichever the URL had.
    pub channel_id: String,
    /// Pretty display label for preview output (defaults to channel_id).
    pub channel_name: String,
}

impl Candidate {
    pub fn into_auto_record(self) -> AutoRecordEntry {
        AutoRecordEntry {
            platform: self.platform,
            channel_id: self.channel_id.clone(),
            channel_name: self.channel_name,
            format: None,
            profile: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ObsExport {
    #[serde(default)]
    sources: Vec<ObsSource>,
}

#[derive(Debug, Deserialize)]
struct ObsSource {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    settings: Option<serde_json::Value>,
}

pub fn parse_obs_export(path: &Path) -> Result<Vec<Candidate>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read OBS export {}", path.display()))?;
    let export: ObsExport =
        serde_json::from_str(&raw).context("parse OBS scene collection JSON")?;

    let mut out = Vec::new();
    for src in &export.sources {
        let id = src.id.as_deref().unwrap_or("");
        // Limit which source kinds we trust to carry a stream URL.
        if !matches!(
            id,
            "ffmpeg_source" | "rtmp_source" | "browser_source" | "vlc_source"
        ) {
            continue;
        }
        let Some(ref settings) = src.settings else {
            continue;
        };
        let url = settings
            .get("input")
            .or_else(|| settings.get("url"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if url.is_empty() {
            continue;
        }
        if let Some(c) = candidate_from_url(url, src.name.as_deref()) {
            out.push(c);
        }
    }
    dedupe(&mut out);
    Ok(out)
}

pub fn parse_streamlink_lines(path: &Path) -> Result<Vec<Candidate>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read streamlink config {}", path.display()))?;
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Streamlink `streams.txt` typically has "twitch.tv/slug" or
        // a full URL per line. `config` files may have key=value lines
        // that include a URL; we just scan the raw line for substrings
        // a platform helper recognizes.
        if let Some(c) = candidate_from_url(trimmed, None) {
            out.push(c);
        }
    }
    dedupe(&mut out);
    Ok(out)
}

/// Recognize twitch / youtube / patreon URLs and convert to a
/// [`Candidate`]. Accepts both full URLs and bare host/slug forms.
pub fn candidate_from_url(raw: &str, friendly_name: Option<&str>) -> Option<Candidate> {
    let lower = raw.to_lowercase();

    if let Some(slug) = extract_twitch(&lower, raw) {
        return Some(Candidate {
            platform: "Twitch".into(),
            channel_id: slug.clone(),
            channel_name: friendly_name.map(str::to_string).unwrap_or(slug),
        });
    }
    if let Some((id, name)) = extract_youtube(&lower, raw) {
        return Some(Candidate {
            platform: "YouTube".into(),
            channel_id: id,
            channel_name: friendly_name.map(str::to_string).unwrap_or(name),
        });
    }
    if let Some(slug) = extract_patreon(&lower, raw) {
        return Some(Candidate {
            platform: "Patreon".into(),
            channel_id: slug.clone(),
            channel_name: friendly_name.map(str::to_string).unwrap_or(slug),
        });
    }
    None
}

fn extract_twitch(lower: &str, raw: &str) -> Option<String> {
    let needle = "twitch.tv/";
    let pos = lower.find(needle)?;
    let tail = &raw[pos + needle.len()..];
    let slug: String = tail
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

fn extract_youtube(lower: &str, raw: &str) -> Option<(String, String)> {
    // /channel/UC… → use the UC ID. /@handle → use the handle.
    if let Some(pos) = lower.find("youtube.com/channel/") {
        let tail = &raw[pos + "youtube.com/channel/".len()..];
        let id: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if !id.is_empty() {
            return Some((id.clone(), id));
        }
    }
    if let Some(pos) = lower.find("youtube.com/@") {
        let tail = &raw[pos + "youtube.com/@".len()..];
        let handle: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .collect();
        if !handle.is_empty() {
            return Some((handle.clone(), handle));
        }
    }
    None
}

fn extract_patreon(lower: &str, raw: &str) -> Option<String> {
    let needle = "patreon.com/";
    let pos = lower.find(needle)?;
    let tail = &raw[pos + needle.len()..];
    let slug: String = tail
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if slug.is_empty() || matches!(slug.as_str(), "posts" | "search" | "home" | "join") {
        None
    } else {
        Some(slug)
    }
}

fn dedupe(out: &mut Vec<Candidate>) {
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    out.retain(|c| seen.insert((c.platform.clone(), c.channel_id.clone())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitch_extraction() {
        let c = candidate_from_url("https://twitch.tv/shroud", None).unwrap();
        assert_eq!(c.platform, "Twitch");
        assert_eq!(c.channel_id, "shroud");
    }

    #[test]
    fn youtube_channel_id() {
        let c = candidate_from_url(
            "https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ",
            None,
        )
        .unwrap();
        assert_eq!(c.platform, "YouTube");
        assert_eq!(c.channel_id, "UCBJycsmduvYEL83R_U4JriQ");
    }

    #[test]
    fn youtube_handle() {
        let c = candidate_from_url("https://youtube.com/@mkbhd", None).unwrap();
        assert_eq!(c.channel_id, "mkbhd");
    }

    #[test]
    fn patreon_url() {
        let c = candidate_from_url("https://patreon.com/somecreator", None).unwrap();
        assert_eq!(c.platform, "Patreon");
        assert_eq!(c.channel_id, "somecreator");
    }

    #[test]
    fn patreon_filters_navigational_urls() {
        assert!(candidate_from_url("https://patreon.com/home", None).is_none());
        assert!(candidate_from_url("https://patreon.com/posts", None).is_none());
    }

    #[test]
    fn unknown_host_returns_none() {
        assert!(candidate_from_url("https://example.com/whatever", None).is_none());
    }

    #[test]
    fn streamlink_parses_inline_lines() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "# header comment\nhttps://twitch.tv/xqc\n\nhttps://youtube.com/@mkbhd\n",
        )
        .unwrap();
        let cands = parse_streamlink_lines(tmp.path()).unwrap();
        assert_eq!(cands.len(), 2);
        assert!(cands.iter().any(|c| c.channel_id == "xqc"));
        assert!(cands.iter().any(|c| c.channel_id == "mkbhd"));
    }

    #[test]
    fn dedupe_skips_repeats() {
        let mut v = vec![
            Candidate {
                platform: "Twitch".into(),
                channel_id: "a".into(),
                channel_name: "a".into(),
            },
            Candidate {
                platform: "Twitch".into(),
                channel_id: "a".into(),
                channel_name: "a".into(),
            },
            Candidate {
                platform: "Twitch".into(),
                channel_id: "b".into(),
                channel_name: "b".into(),
            },
        ];
        dedupe(&mut v);
        assert_eq!(v.len(), 2);
    }
}
