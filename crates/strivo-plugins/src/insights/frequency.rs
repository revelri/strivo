//! Word frequency queries over the Crunchr DB. Read-only.

use anyhow::Result;
use rusqlite::Connection;

/// One row in the frequency view: word + count.
#[derive(Debug, Clone)]
pub struct FrequencyRow {
    pub word: String,
    pub count: i64,
}

/// Curated English stopword list. Small enough to inline; loosely follows
/// the NLTK stopword set's "common, near-meaningless" filler words.
/// Toggle via `[s]` in the Insights pane.
pub const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "if", "of", "to", "in", "on", "at", "for", "from",
    "with", "without", "by", "as", "is", "was", "were", "be", "been", "being", "are", "am",
    "i", "you", "he", "she", "it", "we", "they", "me", "him", "her", "us", "them", "this",
    "that", "these", "those", "my", "your", "his", "their", "our", "its", "do", "does", "did",
    "have", "has", "had", "will", "would", "should", "could", "can", "may", "might", "must",
    "what", "when", "where", "why", "how", "who", "which", "than", "then", "so", "just",
    "uh", "um", "yeah", "like", "okay", "right", "really", "actually", "kind", "sort",
    "very", "much", "many", "any", "all", "some", "no", "not", "only", "even", "also",
    "there", "here", "now", "well", "still", "more", "most", "back", "good", "great",
    "go", "get", "got", "going", "make", "made", "see", "look", "think", "know", "want",
    "say", "said", "tell", "told", "come", "came", "take", "took", "give", "gave",
];

fn stopword_set() -> std::collections::HashSet<&'static str> {
    STOPWORDS.iter().copied().collect()
}

/// Global frequency aggregate across every indexed recording.
pub fn top_words_global(
    conn: &Connection,
    limit: usize,
    include_stopwords: bool,
) -> Result<Vec<FrequencyRow>> {
    let mut stmt = conn.prepare(
        "SELECT word, SUM(count) AS total \
         FROM word_frequency \
         GROUP BY word \
         ORDER BY total DESC \
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64 * 4], |row| {
        Ok(FrequencyRow {
            word: row.get(0)?,
            count: row.get(1)?,
        })
    })?;
    let mut out: Vec<FrequencyRow> = Vec::new();
    let stop = stopword_set();
    for r in rows {
        let row = r?;
        if !include_stopwords && stop.contains(row.word.to_lowercase().as_str()) {
            continue;
        }
        out.push(row);
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

/// Per-recording frequency. `recording_id` is the recording uuid; Crunchr
/// stores it as the `recording_id` text on the `videos` table.
pub fn top_words_for_recording(
    conn: &Connection,
    recording_id: &str,
    limit: usize,
    include_stopwords: bool,
) -> Result<Vec<FrequencyRow>> {
    let mut stmt = conn.prepare(
        "SELECT wf.word, wf.count \
         FROM word_frequency wf \
         JOIN videos v ON v.id = wf.video_id \
         WHERE v.recording_id = ?1 \
         ORDER BY wf.count DESC \
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![recording_id, limit as i64 * 4], |row| {
        Ok(FrequencyRow {
            word: row.get(0)?,
            count: row.get(1)?,
        })
    })?;
    let mut out: Vec<FrequencyRow> = Vec::new();
    let stop = stopword_set();
    for r in rows {
        let row = r?;
        if !include_stopwords && stop.contains(row.word.to_lowercase().as_str()) {
            continue;
        }
        out.push(row);
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopwords_include_common_fillers() {
        let s = stopword_set();
        assert!(s.contains("the"));
        assert!(s.contains("uh"));
        assert!(s.contains("um"));
    }

    #[test]
    fn stopwords_exclude_content_words() {
        let s = stopword_set();
        assert!(!s.contains("stream"));
        assert!(!s.contains("recording"));
    }
}
