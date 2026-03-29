use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Result;
use uuid::Uuid;

use super::types::{PipelineEvent, Segment};

/// Extract audio from an MKV recording using ffmpeg.
pub async fn extract_audio(recording_id: Uuid, video_path: PathBuf, output_dir: PathBuf) -> Box<dyn std::any::Any + Send> {
    match extract_audio_inner(&video_path, &output_dir).await {
        Ok(audio_path) => Box::new(PipelineEvent::AudioExtracted {
            recording_id,
            audio_path,
        }),
        Err(e) => Box::new(PipelineEvent::StageError {
            recording_id,
            error: format!("Audio extraction failed: {e}"),
        }),
    }
}

async fn extract_audio_inner(video_path: &Path, output_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(output_dir)?;

    let stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let audio_path = output_dir.join(format!("{stem}.wav"));

    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-i",
            video_path.to_str().unwrap_or(""),
            "-vn",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-y",
            audio_path.to_str().unwrap_or(""),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffmpeg exited with {}: {}", output.status, stderr.chars().take(200).collect::<String>());
    }

    Ok(audio_path)
}


/// Segment chunker: 512-token target with sentence-boundary splitting
pub fn chunk_segments(segments: &[Segment], target_tokens: usize) -> Vec<Chunk> {
    if segments.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    // Use owned Strings so we can carry remainder text across iterations
    let mut current_texts: Vec<String> = Vec::new();
    let mut current_start = segments[0].start_sec;
    let mut current_tokens: usize = 0;

    for seg in segments {
        let seg_text = seg.text.trim();
        if seg_text.is_empty() {
            continue;
        }

        let seg_tokens = estimate_tokens(seg_text);

        // If adding this segment exceeds target and we have content, finalize chunk
        if current_tokens + seg_tokens > (target_tokens as f64 * 1.2) as usize && !current_texts.is_empty() {
            let chunk_text = normalize_text(&current_texts.join(" "));
            chunks.push(Chunk {
                text: chunk_text,
                start_sec: current_start,
                end_sec: seg.start_sec,
                token_count: current_tokens,
            });
            current_texts.clear();
            current_start = seg.start_sec;
            current_tokens = 0;
        }

        current_texts.push(seg_text.to_string());
        current_tokens += seg_tokens;

        // If we've hit the target, try to break at sentence boundary
        if current_tokens >= target_tokens {
            let combined = normalize_text(&current_texts.join(" "));
            let sentences = split_sentences(&combined);

            if sentences.len() > 1 {
                let mut running = 0;
                let mut break_idx = sentences.len();
                for (i, sent) in sentences.iter().enumerate() {
                    running += estimate_tokens(sent);
                    if running >= target_tokens {
                        break_idx = i + 1;
                        break;
                    }
                }

                let chunk_text = normalize_text(&sentences[..break_idx].join(" "));
                let remainder_text = normalize_text(&sentences[break_idx..].join(" "));

                chunks.push(Chunk {
                    text: chunk_text.clone(),
                    start_sec: current_start,
                    end_sec: seg.end_sec,
                    token_count: estimate_tokens(&chunk_text),
                });

                // Carry remainder into next iteration
                if !remainder_text.is_empty() {
                    current_texts = vec![remainder_text.clone()];
                    current_start = seg.end_sec;
                    current_tokens = estimate_tokens(&remainder_text);
                } else {
                    current_texts.clear();
                    current_start = seg.end_sec;
                    current_tokens = 0;
                }
            } else {
                chunks.push(Chunk {
                    text: combined,
                    start_sec: current_start,
                    end_sec: seg.end_sec,
                    token_count: current_tokens,
                });
                current_texts.clear();
                current_start = seg.end_sec;
                current_tokens = 0;
            }
        }
    }

    // Final chunk
    if !current_texts.is_empty() {
        let chunk_text = normalize_text(&current_texts.join(" "));
        let tokens = estimate_tokens(&chunk_text);
        chunks.push(Chunk {
            text: chunk_text,
            start_sec: current_start,
            end_sec: segments.last().map(|s| s.end_sec).unwrap_or(0.0),
            token_count: tokens,
        });
    }

    chunks
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub text: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub token_count: usize,
}

/// Estimate token count (~words / 0.75).
pub fn estimate_tokens(text: &str) -> usize {
    let words = text.split_whitespace().count();
    ((words as f64) / 0.75).ceil() as usize
}

/// Normalize text: collapse whitespace.
pub fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Split text into sentences on .!? boundaries.
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if (ch == '.' || ch == '!' || ch == '?') && !current.trim().is_empty() {
            sentences.push(current.trim().to_string());
            current.clear();
        }
    }

    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }

    if sentences.is_empty() && !text.is_empty() {
        sentences.push(text.to_string());
    }

    sentences
}

/// Compute word frequencies from text (filtered for stopwords).
pub fn word_frequencies(text: &str) -> Vec<(String, usize)> {
    use rust_stemmers::{Algorithm, Stemmer};
    use std::collections::HashMap;

    let stemmer = Stemmer::create(Algorithm::English);
    let mut freq: HashMap<String, usize> = HashMap::new();

    for word in text.split_whitespace() {
        let lower = word.to_lowercase();
        let cleaned: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
        if cleaned.len() < 2 || is_stopword(&cleaned) {
            continue;
        }
        let stemmed = stemmer.stem(&cleaned).to_string();
        *freq.entry(stemmed).or_insert(0) += 1;
    }

    let mut sorted: Vec<_> = freq.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted
}

fn is_stopword(word: &str) -> bool {
    const STOPWORDS: &[&str] = &[
        "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for",
        "of", "with", "by", "from", "is", "it", "that", "this", "was", "are",
        "be", "have", "has", "had", "do", "does", "did", "will", "would",
        "could", "should", "may", "might", "can", "shall", "not", "no",
        "if", "then", "else", "so", "as", "up", "out", "about", "into",
        "over", "after", "before", "between", "under", "above", "below",
        "all", "each", "every", "both", "few", "more", "most", "other",
        "some", "such", "only", "own", "same", "than", "too", "very",
        "just", "because", "through", "during", "while", "also", "back",
        "been", "being", "here", "there", "when", "where", "which", "who",
        "whom", "what", "how", "its", "my", "your", "his", "her", "our",
        "their", "them", "they", "we", "you", "he", "she", "me", "him",
        "us", "im", "ive", "dont", "youre", "youve", "were", "weve",
    ];
    STOPWORDS.contains(&word)
}

/// Check if the whisper CLI is available.
pub fn is_whisper_available() -> bool {
    std::process::Command::new("whisper")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_segment(index: usize, start: f64, end: f64, text: &str) -> Segment {
        Segment {
            index,
            start_sec: start,
            end_sec: end,
            text: text.to_string(),
            speaker: None,
            confidence: None,
        }
    }

    #[test]
    fn chunk_segments_empty() {
        let chunks = chunk_segments(&[], 512);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_segments_single_short() {
        let segs = vec![make_segment(0, 0.0, 5.0, "Hello world this is a test.")];
        let chunks = chunk_segments(&segs, 512);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("Hello"));
    }

    #[test]
    fn chunk_segments_respects_target() {
        // Create enough segments to exceed target
        let mut segs = Vec::new();
        for i in 0..100 {
            let start = i as f64 * 2.0;
            segs.push(make_segment(i, start, start + 2.0, "This is a somewhat longer segment with several words in it."));
        }
        let chunks = chunk_segments(&segs, 50);
        assert!(chunks.len() > 1);

        // All text should be preserved (no data loss)
        let total_text: String = chunks.iter().map(|c| c.text.clone()).collect::<Vec<_>>().join(" ");
        assert!(total_text.contains("somewhat longer"));
    }

    #[test]
    fn chunk_segments_remainder_preserved() {
        // Regression test: remainder text after sentence-split must not be dropped
        let segs = vec![
            make_segment(0, 0.0, 10.0, "First sentence here. Second sentence here. Third sentence here. Fourth sentence here. Fifth sentence here."),
        ];
        let chunks = chunk_segments(&segs, 10); // Very low target to force splitting
        let all_text: String = chunks.iter().map(|c| c.text.clone()).collect::<Vec<_>>().join(" ");
        assert!(all_text.contains("Fifth"), "Remainder text was dropped! Got: {all_text}");
    }

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens("hello world"), 3); // 2 words / 0.75 = 2.67 -> 3
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("one"), 2); // 1 / 0.75 = 1.33 -> 2
    }

    #[test]
    fn split_sentences_multiple() {
        let result = split_sentences("Hello there. How are you? I'm fine!");
        assert_eq!(result.len(), 3);
        assert!(result[0].starts_with("Hello"));
        assert!(result[1].starts_with("How"));
        assert!(result[2].starts_with("I'm") || result[2].contains("fine"));
    }

    #[test]
    fn split_sentences_no_punctuation() {
        let result = split_sentences("no punctuation here at all");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn word_frequencies_basic() {
        let result = word_frequencies("hello hello world world world test");
        assert!(!result.is_empty());
        // "world" should be stemmed and have highest count
        let (top_word, top_count) = &result[0];
        assert_eq!(*top_count, 3);
        assert_eq!(top_word, "world");
    }

    #[test]
    fn word_frequencies_filters_stopwords() {
        let result = word_frequencies("the and or but is are was");
        assert!(result.is_empty(), "Stopwords should be filtered");
    }

    #[test]
    fn word_frequencies_empty() {
        let result = word_frequencies("");
        assert!(result.is_empty());
    }

    #[test]
    fn normalize_text_collapses_whitespace() {
        assert_eq!(normalize_text("  hello   world  "), "hello world");
    }
}
