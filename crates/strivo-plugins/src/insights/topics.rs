//! Cross-recording topic graph queries. (I3.)
//!
//! Crunchr's `video_analysis.topics` column stores an LLM-extracted
//! topic list per video as a JSON array of strings (or sometimes a
//! single comma-separated string from earlier analysis runs). We
//! aggregate across every analyzed recording so the user sees which
//! topics recur and when they first appeared.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

/// One row in the topic-graph view.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicRow {
    pub topic: String,
    pub count: i64,
    /// Earliest `videos.created_at` (date-only) the topic was seen.
    pub first_seen: String,
    /// Latest `videos.created_at` (date-only) the topic was seen.
    pub last_seen: String,
}

/// Walk `video_analysis.topics` across every analyzed recording and
/// return one row per distinct topic. Topics are normalized: trimmed,
/// lowercased, deduplicated within a single video.
pub fn cross_recording_topics(conn: &Connection) -> Result<Vec<TopicRow>> {
    let mut stmt = conn.prepare(
        "SELECT va.topics, date(v.created_at) AS day \
         FROM video_analysis va \
         JOIN videos v ON v.id = va.video_id \
         WHERE va.topics IS NOT NULL AND va.topics != ''",
    )?;
    let rows = stmt.query_map([], |row| {
        let topics: String = row.get(0)?;
        let day: String = row.get(1)?;
        Ok((topics, day))
    })?;

    let mut by_topic: HashMap<String, TopicRow> = HashMap::new();
    for r in rows {
        let (raw, day) = r?;
        let topics = parse_topics(&raw);
        let mut seen_in_video: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for t in topics {
            let key = t.trim().to_lowercase();
            if key.is_empty() || !seen_in_video.insert(key.clone()) {
                continue;
            }
            by_topic
                .entry(key.clone())
                .and_modify(|row| {
                    row.count += 1;
                    if day.as_str() < row.first_seen.as_str() {
                        row.first_seen = day.clone();
                    }
                    if day.as_str() > row.last_seen.as_str() {
                        row.last_seen = day.clone();
                    }
                })
                .or_insert(TopicRow {
                    topic: key,
                    count: 1,
                    first_seen: day.clone(),
                    last_seen: day.clone(),
                });
        }
    }
    let mut out: Vec<TopicRow> = by_topic.into_values().collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.topic.cmp(&b.topic)));
    Ok(out)
}

/// Parse a topics field — handles both JSON-array and
/// comma-separated formats.
fn parse_topics(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
            return arr;
        }
        // Sometimes the LLM emits `[{"name": "..."}]`; accept those too.
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
            return arr
                .into_iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(String::from)
                        .or_else(|| v.get("name").and_then(|n| n.as_str()).map(String::from))
                        .or_else(|| {
                            v.get("topic").and_then(|n| n.as_str()).map(String::from)
                        })
                })
                .collect();
        }
    }
    trimmed.split(',').map(|s| s.trim().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_topics_json_array() {
        let out = parse_topics(r#"["streaming","retro gaming","speedrun"]"#);
        assert_eq!(out, vec!["streaming", "retro gaming", "speedrun"]);
    }

    #[test]
    fn parse_topics_csv_fallback() {
        let out = parse_topics("streaming, retro gaming , speedrun");
        assert_eq!(out, vec!["streaming", "retro gaming", "speedrun"]);
    }

    #[test]
    fn parse_topics_object_array() {
        let out =
            parse_topics(r#"[{"name":"streaming"},{"topic":"retro gaming"}]"#);
        assert_eq!(out, vec!["streaming", "retro gaming"]);
    }
}
