//! Speaker-filter cut. (E4.)
//!
//! Turns a transcript's segments + a speaker rule into a clip list that
//! drops (or keeps only) listed speakers. The resulting clips feed the
//! lossless concat path the way any other Editor clip list would.

use crate::crunchr::types::Segment;
use super::EditorClip;

#[derive(Debug, Clone)]
pub struct SpeakerFilter {
    /// Speaker labels to act on. The exact match is case-insensitive.
    pub speakers: Vec<String>,
    /// `false` = drop these speakers, keep the others (default).
    /// `true`  = keep only these speakers, drop the rest.
    pub keep_only: bool,
}

impl SpeakerFilter {
    /// Build a clip list from `segments`. Each contiguous run of
    /// kept segments becomes one clip; runs collapse so the user gets
    /// one clip per kept run rather than one per segment.
    ///
    /// `word_index_for_seg` maps a segment index → the word position
    /// where that segment starts in the flattened word stream. The
    /// Editor passes this from the same source that drives the
    /// timeline view, so clip in/out indices land on real word
    /// boundaries.
    pub fn apply(
        &self,
        segments: &[Segment],
        word_index_for_seg: impl Fn(usize) -> u32,
    ) -> Vec<EditorClip> {
        let lower: Vec<String> = self
            .speakers
            .iter()
            .map(|s| s.to_lowercase())
            .collect();

        let keep = |seg: &Segment| -> bool {
            let Some(label) = &seg.speaker else {
                // Segments without a speaker label only survive when we're
                // dropping a specific set (the user can't be targeting an
                // unknown speaker).
                return !self.keep_only;
            };
            let matched = lower.iter().any(|s| s == &label.to_lowercase());
            if self.keep_only {
                matched
            } else {
                !matched
            }
        };

        let mut clips: Vec<EditorClip> = Vec::new();
        let mut run_start: Option<usize> = None;
        for (i, seg) in segments.iter().enumerate() {
            if keep(seg) {
                if run_start.is_none() {
                    run_start = Some(i);
                }
            } else if let Some(start) = run_start.take() {
                clips.push(EditorClip {
                    in_word: word_index_for_seg(start),
                    out_word: word_index_for_seg(i),
                    label: format!("{}..{}", segments[start].text.split_whitespace().take(3).collect::<Vec<_>>().join(" "), segments[i - 1].text.split_whitespace().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ")),
                });
            }
        }
        // Trailing run.
        if let Some(start) = run_start {
            let n = segments.len();
            clips.push(EditorClip {
                in_word: word_index_for_seg(start),
                out_word: word_index_for_seg(n),
                label: format!(
                    "{}..{}",
                    segments[start]
                        .text
                        .split_whitespace()
                        .take(3)
                        .collect::<Vec<_>>()
                        .join(" "),
                    segments[n - 1]
                        .text
                        .split_whitespace()
                        .rev()
                        .take(3)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            });
        }
        clips
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crunchr::types::Segment;

    fn s(speaker: Option<&str>, text: &str) -> Segment {
        Segment {
            speaker: speaker.map(String::from),
            text: text.to_string(),
            ..Segment::default()
        }
    }

    #[test]
    fn drop_speaker_keeps_alternating() {
        // A-B-A-B → drop Bob → two clips: seg 0 alone, seg 2 alone.
        let segs = vec![
            s(Some("Alice"), "hi there"),
            s(Some("Bob"), "yeah whatever"),
            s(Some("Alice"), "anyway as I was saying"),
            s(Some("Bob"), "OK"),
        ];
        let f = SpeakerFilter {
            speakers: vec!["Bob".into()],
            keep_only: false,
        };
        // word_index = 100 * segment index for the test (so we can
        // verify clip ranges without a real word stream).
        let clips = f.apply(&segs, |i| (i * 100) as u32);
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].in_word, 0);
        assert_eq!(clips[0].out_word, 100);
        assert_eq!(clips[1].in_word, 200);
        assert_eq!(clips[1].out_word, 300);
    }

    #[test]
    fn keep_only_inverts() {
        let segs = vec![
            s(Some("Alice"), "hi"),
            s(Some("Bob"), "yo"),
            s(Some("Alice"), "ok"),
        ];
        let f = SpeakerFilter {
            speakers: vec!["alice".into()], // case-insensitive
            keep_only: true,
        };
        let clips = f.apply(&segs, |i| i as u32);
        // Two singleton runs.
        assert_eq!(clips.len(), 2);
    }

    #[test]
    fn contiguous_run_collapses() {
        let segs = vec![
            s(Some("Alice"), "a"),
            s(Some("Alice"), "b"),
            s(Some("Bob"), "c"),
            s(Some("Alice"), "d"),
        ];
        let f = SpeakerFilter {
            speakers: vec!["Bob".into()],
            keep_only: false,
        };
        let clips = f.apply(&segs, |i| i as u32);
        // 0..2 and 3..4 — two clips, not four.
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].in_word, 0);
        assert_eq!(clips[0].out_word, 2);
        assert_eq!(clips[1].in_word, 3);
        assert_eq!(clips[1].out_word, 4);
    }

    #[test]
    fn unlabeled_segments_pass_under_drop() {
        let segs = vec![
            s(None, "narrator-ish"),
            s(Some("Alice"), "hi"),
        ];
        let f = SpeakerFilter {
            speakers: vec!["Alice".into()],
            keep_only: false,
        };
        let clips = f.apply(&segs, |i| i as u32);
        // Only the unlabeled segment survives.
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].in_word, 0);
        assert_eq!(clips[0].out_word, 1);
    }

    #[test]
    fn unlabeled_segments_dropped_under_keep_only() {
        let segs = vec![
            s(None, "narrator-ish"),
            s(Some("Alice"), "hi"),
        ];
        let f = SpeakerFilter {
            speakers: vec!["Alice".into()],
            keep_only: true,
        };
        let clips = f.apply(&segs, |i| i as u32);
        // Only the Alice segment survives.
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].in_word, 1);
        assert_eq!(clips[0].out_word, 2);
    }
}
