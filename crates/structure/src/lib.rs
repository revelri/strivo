//! Structure analyzer — DAW-style section markers for a stream.
//!
//! DAWs label sections (intro · verse · chorus · bridge · outro);
//! streamers want the same on long captures so editors / casebook
//! briefings can jump straight to "the gameplay segment" or "the chat
//! break" without scrubbing.
//!
//! Pure-data composer: takes a [`StructureInputs`] (chapter spans + chat
//! density buckets + scene cuepoints + total duration) and emits a flat
//! list of non-overlapping [`Segment`]s tagged with a [`SectionKind`]:
//!
//! * **Intro** — opens the broadcast; low chat + scene density.
//! * **Outro** — closes the broadcast; pacing falls off.
//! * **Break** — sustained dip in chat density AND scene rate
//!   in the middle of the broadcast.
//! * **Gameplay** — high chat density + steady scene rate.
//! * **Content** — everything else (catch-all so segments tile the
//!   timeline without gaps).
//!
//! The host doesn't have to provide every input — missing fields fall
//! back to the catch-all. The output always tiles the full duration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    Intro,
    Outro,
    Break,
    Gameplay,
    Content,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub kind: SectionKind,
    pub start_sec: f32,
    pub end_sec: f32,
    /// Optional human-readable label the host can override (e.g. the
    /// matching chapter title when one exists for the same span).
    pub label: Option<String>,
}

impl Segment {
    pub fn duration(&self) -> f32 {
        (self.end_sec - self.start_sec).max(0.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterSpan {
    pub title: String,
    pub start_sec: f32,
    pub end_sec: f32,
}

/// One bucket from the chat-density plugin's heatmap output.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChatBucket {
    pub start_sec: f32,
    pub end_sec: f32,
    /// Messages per minute in this bucket.
    pub rate_mpm: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StructureInputs {
    pub total_duration_sec: f32,
    #[serde(default)]
    pub chapters: Vec<ChapterSpan>,
    #[serde(default)]
    pub chat_buckets: Vec<ChatBucket>,
    /// Scene-cut timestamps in seconds.
    #[serde(default)]
    pub scene_cuts_sec: Vec<f32>,
}

/// Tunables — exposed so the host UI can let power users tweak per
/// channel without recompiling. Defaults work for the typical "Twitch
/// just-chatting / gameplay" broadcast.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ClassifierKnobs {
    /// Anything shorter than this from t=0 collapses to a single Intro.
    pub intro_window_sec: f32,
    /// Anything shorter than this before total_duration collapses to Outro.
    pub outro_window_sec: f32,
    /// Bucket-rate below which we treat the run as a Break.
    pub break_chat_mpm: f32,
    /// Above this msgs/min the bucket is considered Gameplay-busy.
    pub gameplay_chat_mpm: f32,
    /// Minimum contiguous duration to qualify as a Break — short dips
    /// (ad breaks, momentary silence) shouldn't earn their own segment.
    pub min_break_sec: f32,
}

impl Default for ClassifierKnobs {
    fn default() -> Self {
        Self {
            intro_window_sec: 180.0,    // first 3 minutes
            outro_window_sec: 120.0,    // last 2 minutes
            break_chat_mpm: 8.0,
            gameplay_chat_mpm: 30.0,
            min_break_sec: 90.0,
        }
    }
}

/// Run the classifier. The output always tiles `[0, total_duration_sec]`
/// without gaps or overlaps so downstream consumers (casebook briefing,
/// editor scrubbing) can iterate without bounds checks.
pub fn classify(inputs: &StructureInputs, knobs: &ClassifierKnobs) -> Vec<Segment> {
    let total = inputs.total_duration_sec.max(0.0);
    if total <= 0.0 {
        return vec![];
    }
    // Step 1: lay down the structural spine — intro window, outro window,
    // and whatever Break runs fall inside the middle.
    let mut anchors: Vec<Segment> = Vec::new();

    // Intro slot: 0 → min(intro_window, total).
    let intro_end = knobs.intro_window_sec.min(total);
    if intro_end > 0.0 {
        anchors.push(Segment {
            kind: SectionKind::Intro,
            start_sec: 0.0,
            end_sec: intro_end,
            label: Some("Intro".into()),
        });
    }

    // Outro slot: total - outro_window → total. Skip when the broadcast
    // is too short for both windows to fit without overlap.
    let outro_start = (total - knobs.outro_window_sec).max(0.0);
    if outro_start > intro_end {
        anchors.push(Segment {
            kind: SectionKind::Outro,
            start_sec: outro_start,
            end_sec: total,
            label: Some("Outro".into()),
        });
    }

    // Break runs inside the middle. Walk chat buckets, fold into runs
    // whose rate stays below break_chat_mpm. A run earns the Break label
    // when its duration ≥ min_break_sec.
    let mid_lo = intro_end;
    let mid_hi = outro_start.max(intro_end);
    let mut breaks: Vec<Segment> = Vec::new();
    if mid_hi > mid_lo {
        let mut run_start: Option<f32> = None;
        let mut run_end: f32 = 0.0;
        for b in &inputs.chat_buckets {
            let inside_lo = b.start_sec.max(mid_lo);
            let inside_hi = b.end_sec.min(mid_hi);
            if inside_hi <= inside_lo {
                continue;
            }
            // Scene-rate veto: even if chat is dead, plentiful scene cuts
            // mean the broadcaster is actively on-screen (gameplay /
            // demo with audio off / silent solo run). Don't tag those as
            // Break.
            let scene_busy = midspan_scene_rate(&inputs.scene_cuts_sec, inside_lo, inside_hi) >= 3.0;
            if b.rate_mpm <= knobs.break_chat_mpm && !scene_busy {
                run_start = run_start.or(Some(inside_lo));
                run_end = inside_hi;
            } else if let Some(s) = run_start.take() {
                if run_end - s >= knobs.min_break_sec {
                    breaks.push(Segment {
                        kind: SectionKind::Break,
                        start_sec: s,
                        end_sec: run_end,
                        label: Some("Break".into()),
                    });
                }
            }
        }
        if let Some(s) = run_start {
            if run_end - s >= knobs.min_break_sec {
                breaks.push(Segment {
                    kind: SectionKind::Break,
                    start_sec: s,
                    end_sec: run_end,
                    label: Some("Break".into()),
                });
            }
        }
    }
    // Merge anchors + breaks then tile gaps with Gameplay / Content.
    anchors.extend(breaks);
    anchors.sort_by(|a, b| a.start_sec.partial_cmp(&b.start_sec).unwrap_or(std::cmp::Ordering::Equal));

    let mut out: Vec<Segment> = Vec::new();
    let mut cursor = 0.0f32;
    for seg in anchors {
        if seg.start_sec > cursor {
            // Gap — fill with Gameplay if the chat density says busy,
            // else generic Content. Use the cuepoint density as a
            // tiebreaker so "talking with steady scene rate" still
            // reads as gameplay-paced.
            let kind = if midspan_is_busy(&inputs.chat_buckets, &inputs.scene_cuts_sec, knobs.gameplay_chat_mpm, cursor, seg.start_sec) {
                SectionKind::Gameplay
            } else {
                SectionKind::Content
            };
            out.push(Segment { kind, start_sec: cursor, end_sec: seg.start_sec, label: None });
        }
        cursor = seg.end_sec.max(cursor);
        out.push(seg);
    }
    if cursor < total {
        let kind = if midspan_is_busy(&inputs.chat_buckets, &inputs.scene_cuts_sec, knobs.gameplay_chat_mpm, cursor, total) {
            SectionKind::Gameplay
        } else {
            SectionKind::Content
        };
        out.push(Segment { kind, start_sec: cursor, end_sec: total, label: None });
    }

    // Step 2: enrich gameplay/content labels using chapter titles when
    // a chapter exactly contains the segment (helps the briefing reader).
    for seg in out.iter_mut() {
        if seg.label.is_some() {
            continue;
        }
        if let Some(ch) = inputs.chapters.iter().find(|c| c.start_sec <= seg.start_sec + 0.5 && c.end_sec >= seg.end_sec - 0.5) {
            seg.label = Some(ch.title.clone());
        }
    }

    out
}

fn midspan_scene_rate(scenes: &[f32], lo: f32, hi: f32) -> f32 {
    let span = (hi - lo).max(0.001);
    if scenes.is_empty() {
        return 0.0;
    }
    let count = scenes.iter().filter(|t| **t >= lo && **t <= hi).count() as f32;
    (count / span) * 60.0
}

fn midspan_is_busy(buckets: &[ChatBucket], scenes: &[f32], gameplay_mpm: f32, lo: f32, hi: f32) -> bool {
    let span = (hi - lo).max(0.001);
    let avg_chat = if buckets.is_empty() {
        0.0
    } else {
        let mut weighted = 0.0;
        for b in buckets {
            let inside = b.end_sec.min(hi) - b.start_sec.max(lo);
            if inside > 0.0 {
                weighted += b.rate_mpm * inside;
            }
        }
        weighted / span
    };
    let scene_rate_per_min = if scenes.is_empty() {
        0.0
    } else {
        let count = scenes.iter().filter(|t| **t >= lo && **t <= hi).count() as f32;
        (count / span) * 60.0
    };
    // Busy = chat-rate near gameplay threshold OR scene cuts are frequent
    // (3 per minute = sustained on-screen action).
    avg_chat >= gameplay_mpm || scene_rate_per_min >= 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(s: f32, e: f32, mpm: f32) -> ChatBucket {
        ChatBucket { start_sec: s, end_sec: e, rate_mpm: mpm }
    }

    #[test]
    fn empty_inputs_yield_no_segments() {
        let out = classify(&StructureInputs::default(), &ClassifierKnobs::default());
        assert!(out.is_empty());
    }

    #[test]
    fn very_short_capture_collapses_to_intro_only() {
        let inputs = StructureInputs { total_duration_sec: 60.0, ..Default::default() };
        let out = classify(&inputs, &ClassifierKnobs::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, SectionKind::Intro);
        assert_eq!(out[0].end_sec, 60.0);
    }

    #[test]
    fn long_broadcast_gets_intro_outro_and_middle() {
        let inputs = StructureInputs {
            total_duration_sec: 3600.0, // 1 hour
            chat_buckets: vec![bucket(200.0, 3400.0, 40.0)], // busy middle
            ..Default::default()
        };
        let out = classify(&inputs, &ClassifierKnobs::default());
        // 3 segments: intro, gameplay (busy middle), outro.
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].kind, SectionKind::Intro);
        assert_eq!(out[1].kind, SectionKind::Gameplay);
        assert_eq!(out[2].kind, SectionKind::Outro);
    }

    #[test]
    fn sustained_chat_dip_in_middle_becomes_break() {
        // 30 min broadcast. Intro 3 min, outro 2 min. Middle = 25 min.
        // Chat drops to 2 mpm between 600-900 (5 min ≥ min_break 90s).
        let inputs = StructureInputs {
            total_duration_sec: 1800.0,
            chat_buckets: vec![
                bucket(180.0, 600.0, 50.0),
                bucket(600.0, 900.0, 2.0),
                bucket(900.0, 1680.0, 60.0),
            ],
            ..Default::default()
        };
        let out = classify(&inputs, &ClassifierKnobs::default());
        assert!(out.iter().any(|s| s.kind == SectionKind::Break));
        let br = out.iter().find(|s| s.kind == SectionKind::Break).unwrap();
        assert!(br.start_sec >= 600.0 - 0.5 && br.end_sec <= 900.0 + 0.5);
    }

    #[test]
    fn short_chat_dip_does_not_become_break() {
        // 45-sec dip below min_break — should stay gameplay.
        let inputs = StructureInputs {
            total_duration_sec: 1800.0,
            chat_buckets: vec![
                bucket(180.0, 800.0, 60.0),
                bucket(800.0, 845.0, 2.0),
                bucket(845.0, 1680.0, 60.0),
            ],
            ..Default::default()
        };
        let out = classify(&inputs, &ClassifierKnobs::default());
        assert!(!out.iter().any(|s| s.kind == SectionKind::Break));
    }

    #[test]
    fn segments_tile_full_duration_without_gaps() {
        let inputs = StructureInputs {
            total_duration_sec: 3600.0,
            chat_buckets: vec![bucket(0.0, 3600.0, 25.0)],
            scene_cuts_sec: vec![100.0, 200.0, 300.0, 400.0, 500.0],
            ..Default::default()
        };
        let out = classify(&inputs, &ClassifierKnobs::default());
        let mut cursor = 0.0;
        for s in &out {
            assert!((s.start_sec - cursor).abs() < 0.01, "gap at {cursor}");
            cursor = s.end_sec;
        }
        assert!((cursor - 3600.0).abs() < 0.01);
    }

    #[test]
    fn chapter_titles_get_attached_to_gameplay_runs() {
        let inputs = StructureInputs {
            total_duration_sec: 3600.0,
            chat_buckets: vec![bucket(200.0, 3400.0, 50.0)],
            chapters: vec![ChapterSpan {
                title: "Boss fight".into(),
                start_sec: 180.0,
                end_sec: 3480.0,
            }],
            ..Default::default()
        };
        let out = classify(&inputs, &ClassifierKnobs::default());
        let gameplay = out.iter().find(|s| s.kind == SectionKind::Gameplay).unwrap();
        assert_eq!(gameplay.label.as_deref(), Some("Boss fight"));
    }

    #[test]
    fn quiet_middle_with_no_scenes_falls_to_content() {
        let inputs = StructureInputs {
            total_duration_sec: 1800.0,
            chat_buckets: vec![bucket(180.0, 1680.0, 5.0)], // low chat
            ..Default::default()
        };
        let out = classify(&inputs, &ClassifierKnobs::default());
        // No chat → break filter catches the long dip as a Break.
        // The remaining gaps (if any) become Content. Either way no
        // Gameplay should appear.
        assert!(!out.iter().any(|s| s.kind == SectionKind::Gameplay));
    }

    #[test]
    fn high_scene_rate_overrides_low_chat_for_gameplay() {
        let inputs = StructureInputs {
            total_duration_sec: 1800.0,
            chat_buckets: vec![bucket(180.0, 1680.0, 5.0)], // low chat
            // 10 cuts in a 1500-sec span → ~0.4/min — NOT busy.
            scene_cuts_sec: vec![200.0, 300.0, 400.0, 500.0, 600.0, 700.0, 800.0, 900.0, 1000.0, 1100.0],
            ..Default::default()
        };
        let out = classify(&inputs, &ClassifierKnobs::default());
        assert!(!out.iter().any(|s| s.kind == SectionKind::Gameplay));

        // Now make it busy (3+ per min over a 60s window):
        let scenes_busy: Vec<f32> = (0..120).map(|i| 200.0 + (i as f32 * 0.5)).collect();
        let inputs_busy = StructureInputs {
            total_duration_sec: 1800.0,
            chat_buckets: vec![bucket(180.0, 1680.0, 5.0)],
            scene_cuts_sec: scenes_busy,
            ..Default::default()
        };
        let out2 = classify(&inputs_busy, &ClassifierKnobs::default());
        assert!(out2.iter().any(|s| s.kind == SectionKind::Gameplay));
    }

    #[test]
    fn outro_slot_skipped_when_broadcast_too_short_for_both_windows() {
        // 4-minute broadcast — intro alone covers 3 min; remaining 60s
        // is shorter than the outro window's 120s anchor.
        let inputs = StructureInputs { total_duration_sec: 240.0, ..Default::default() };
        let out = classify(&inputs, &ClassifierKnobs::default());
        assert!(out.iter().any(|s| s.kind == SectionKind::Intro));
        assert!(!out.iter().any(|s| s.kind == SectionKind::Outro));
        // Tail filled with Content (default catch-all).
        assert!(out.iter().any(|s| s.kind == SectionKind::Content));
    }

    #[test]
    fn knobs_override_widens_intro_window() {
        let mut knobs = ClassifierKnobs::default();
        knobs.intro_window_sec = 600.0;
        let inputs = StructureInputs { total_duration_sec: 3600.0, ..Default::default() };
        let out = classify(&inputs, &knobs);
        assert_eq!(out[0].kind, SectionKind::Intro);
        assert_eq!(out[0].end_sec, 600.0);
    }

    #[test]
    fn json_roundtrip_preserves_inputs_and_segments() {
        let inputs = StructureInputs {
            total_duration_sec: 1800.0,
            chapters: vec![ChapterSpan { title: "x".into(), start_sec: 10.0, end_sec: 20.0 }],
            chat_buckets: vec![bucket(0.0, 100.0, 25.0)],
            scene_cuts_sec: vec![5.0, 15.0, 25.0],
        };
        let s = serde_json::to_string(&inputs).unwrap();
        let back: StructureInputs = serde_json::from_str(&s).unwrap();
        assert_eq!(back.chapters.len(), 1);
        let segs = classify(&back, &ClassifierKnobs::default());
        let serialised = serde_json::to_string(&segs).unwrap();
        assert!(serialised.contains("\"intro\""));
    }
}
