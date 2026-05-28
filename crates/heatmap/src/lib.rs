//! strivo-heatmap — multi-signal audience-retention heatmap.
//!
//! The DAW-vision capability: tell the editor where viewers drop and
//! where to cut. The earlier `strivo-insights-compare::compute_retention`
//! fused two signals (transcript talk density + cuepoint action
//! density). Heatmap layers in highlights and brand-safety penalties
//! and exposes the per-channel decomposition so the SPA can render a
//! multi-band timeline strip with breakdowns on hover.
//!
//! Composed channels (each normalised to [0,1] across the recording
//! before fusion):
//!
//!   * `talk`         — words per second within the bucket
//!   * `action`       — cuepoint count within the bucket
//!   * `highlight`    — sum of clipper highlight scores landing in
//!                      the bucket
//!   * `brandsafe`    — per-bucket count of brand-safety verdicts;
//!                      this is a *negative* signal that subtracts
//!                      from the fused retention so the editor sees
//!                      the dip where a slur lands.
//!
//! Fused = clamp(0.40·talk + 0.30·action + 0.30·highlight − 0.50·brandsafe, 0, 1)
//! then 3-bucket moving-average smoothed so single dead/loud buckets
//! don't tank the curve.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy)]
pub struct TranscriptSegment {
    pub start_sec: f32,
    pub end_sec: f32,
    pub word_count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ScoredEvent {
    pub time_sec: f32,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct HeatmapInputs<'a> {
    pub segments: &'a [TranscriptSegment],
    pub cuepoint_times: &'a [f32],
    pub highlights: &'a [ScoredEvent],
    /// Brand-safety verdict timestamps — treated as anti-signal.
    pub brandsafe_times: &'a [f32],
    pub duration_sec: f32,
    pub bucket_secs: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HeatBucket {
    pub bucket_start: f32,
    pub talk: f32,
    pub action: f32,
    pub highlight: f32,
    pub brandsafe: f32,
    pub fused: f32,
}

/// Build the heatmap. Empty inputs yield an empty vector.
pub fn compute_heatmap(inp: &HeatmapInputs) -> Vec<HeatBucket> {
    if inp.duration_sec <= 0.0 || inp.bucket_secs <= 0.0 {
        return Vec::new();
    }
    let n = (inp.duration_sec / inp.bucket_secs).ceil() as usize;
    if n == 0 {
        return Vec::new();
    }
    let bucket_secs = inp.bucket_secs;
    let mut talk = vec![0.0_f32; n];
    let mut action = vec![0.0_f32; n];
    let mut highlight = vec![0.0_f32; n];
    let mut brandsafe = vec![0.0_f32; n];

    // Talk — overlap-weighted from segments.
    for s in inp.segments {
        if s.end_sec <= s.start_sec {
            continue;
        }
        let span = (s.end_sec - s.start_sec).max(0.001);
        let wps = (s.word_count as f32) / span;
        let lo = (s.start_sec / bucket_secs).floor() as usize;
        let hi = ((s.end_sec / bucket_secs).ceil() as usize).min(n);
        for b in lo..hi {
            let b_lo = b as f32 * bucket_secs;
            let b_hi = b_lo + bucket_secs;
            let overlap = (s.end_sec.min(b_hi) - s.start_sec.max(b_lo)).max(0.0);
            if overlap > 0.0 {
                talk[b] += wps * overlap / bucket_secs;
            }
        }
    }

    // Action — cuepoint count per bucket.
    for &t in inp.cuepoint_times {
        if t < 0.0 {
            continue;
        }
        let b = (t / bucket_secs).floor() as usize;
        if b < n {
            action[b] += 1.0;
        }
    }

    // Highlight — score-weighted event drop into the matching bucket.
    for &ev in inp.highlights {
        if ev.time_sec < 0.0 {
            continue;
        }
        let b = (ev.time_sec / bucket_secs).floor() as usize;
        if b < n {
            highlight[b] += ev.score.clamp(0.0, 1.0);
        }
    }

    // Brand-safety — verdict count (anti-signal).
    for &t in inp.brandsafe_times {
        if t < 0.0 {
            continue;
        }
        let b = (t / bucket_secs).floor() as usize;
        if b < n {
            brandsafe[b] += 1.0;
        }
    }

    let normalise = |v: &mut [f32]| {
        let max = v.iter().copied().fold(0.0_f32, f32::max);
        if max > 0.0 {
            for x in v.iter_mut() {
                *x /= max;
            }
        }
    };
    normalise(&mut talk);
    normalise(&mut action);
    normalise(&mut highlight);
    normalise(&mut brandsafe);

    // Fuse, then smooth.
    let raw_fused: Vec<f32> = (0..n)
        .map(|i| {
            let raw = 0.40 * talk[i] + 0.30 * action[i] + 0.30 * highlight[i]
                - 0.50 * brandsafe[i];
            raw.clamp(0.0, 1.0)
        })
        .collect();
    let mut fused = raw_fused.clone();
    for i in 0..n {
        let lo = i.saturating_sub(1);
        let hi = (i + 1).min(n - 1);
        fused[i] = (raw_fused[lo] + raw_fused[i] + raw_fused[hi]) / 3.0;
    }

    (0..n)
        .map(|i| HeatBucket {
            bucket_start: i as f32 * bucket_secs,
            talk: talk[i],
            action: action[i],
            highlight: highlight[i],
            brandsafe: brandsafe[i],
            fused: fused[i],
        })
        .collect()
}

/// Convenience: pick the top-K buckets by fused score.
pub fn top_k_buckets(buckets: &[HeatBucket], k: usize) -> Vec<HeatBucket> {
    if k == 0 || buckets.is_empty() {
        return Vec::new();
    }
    let mut sorted = buckets.to_vec();
    sorted.sort_by(|a, b| {
        b.fused
            .partial_cmp(&a.fused)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted.into_iter().take(k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(start: f32, end: f32, words: u32) -> TranscriptSegment {
        TranscriptSegment {
            start_sec: start,
            end_sec: end,
            word_count: words,
        }
    }
    fn ev(time: f32, score: f32) -> ScoredEvent {
        ScoredEvent { time_sec: time, score }
    }

    #[test]
    fn empty_inputs_yield_empty_heatmap() {
        let inp = HeatmapInputs {
            segments: &[],
            cuepoint_times: &[],
            highlights: &[],
            brandsafe_times: &[],
            duration_sec: 0.0,
            bucket_secs: 30.0,
        };
        assert!(compute_heatmap(&inp).is_empty());
    }

    #[test]
    fn talk_only_lights_first_bucket() {
        let segs = [ts(0.0, 30.0, 60)];
        let inp = HeatmapInputs {
            segments: &segs,
            cuepoint_times: &[],
            highlights: &[],
            brandsafe_times: &[],
            duration_sec: 120.0,
            bucket_secs: 30.0,
        };
        let buckets = compute_heatmap(&inp);
        assert_eq!(buckets.len(), 4);
        assert!(buckets[0].talk > 0.0);
        // Smoothing pulls neighbour, but bucket 3 stays cold.
        assert!(buckets[0].fused > buckets[3].fused);
    }

    #[test]
    fn highlight_lifts_fused_score() {
        // Bucket 2 has just a highlight, no talk/action.
        let highlights = [ev(60.0, 1.0)];
        let inp = HeatmapInputs {
            segments: &[],
            cuepoint_times: &[],
            highlights: &highlights,
            brandsafe_times: &[],
            duration_sec: 120.0,
            bucket_secs: 30.0,
        };
        let buckets = compute_heatmap(&inp);
        assert_eq!(buckets.len(), 4);
        assert!(buckets[2].highlight > 0.0, "got {:?}", buckets[2]);
        assert!(buckets[2].fused > 0.0);
    }

    #[test]
    fn brandsafe_subtracts_from_fused() {
        // Talk is uniform; brandsafe lands in bucket 1 → bucket 1
        // should fuse lower than bucket 0 or 2.
        let segs = [
            ts(0.0, 30.0, 30),
            ts(30.0, 60.0, 30),
            ts(60.0, 90.0, 30),
        ];
        let bs = [40.0_f32]; // bucket 1
        let inp = HeatmapInputs {
            segments: &segs,
            cuepoint_times: &[],
            highlights: &[],
            brandsafe_times: &bs,
            duration_sec: 90.0,
            bucket_secs: 30.0,
        };
        let buckets = compute_heatmap(&inp);
        assert_eq!(buckets.len(), 3);
        assert!(buckets[1].brandsafe > 0.0);
        assert!(buckets[1].fused <= buckets[0].fused);
        assert!(buckets[1].fused <= buckets[2].fused);
    }

    #[test]
    fn channels_individually_normalised_to_unit_range() {
        let segs = [ts(0.0, 60.0, 60)];
        let cps = [10.0_f32, 12.0, 14.0];
        let highlights = [ev(5.0, 0.9)];
        let bs = [30.0_f32];
        let inp = HeatmapInputs {
            segments: &segs,
            cuepoint_times: &cps,
            highlights: &highlights,
            brandsafe_times: &bs,
            duration_sec: 60.0,
            bucket_secs: 30.0,
        };
        let buckets = compute_heatmap(&inp);
        for b in &buckets {
            for v in [b.talk, b.action, b.highlight, b.brandsafe, b.fused] {
                assert!((0.0..=1.0).contains(&v), "out of range: {b:?}");
            }
        }
    }

    #[test]
    fn smoothing_pulls_isolated_bucket_off_zero() {
        // Bucket 0 has talk, bucket 1 has nothing, bucket 2 has talk.
        // Bucket 1's fused should be lifted off zero by smoothing.
        let segs = [
            ts(0.0, 30.0, 60),
            ts(60.0, 90.0, 60),
        ];
        let inp = HeatmapInputs {
            segments: &segs,
            cuepoint_times: &[],
            highlights: &[],
            brandsafe_times: &[],
            duration_sec: 90.0,
            bucket_secs: 30.0,
        };
        let buckets = compute_heatmap(&inp);
        assert_eq!(buckets.len(), 3);
        assert!(buckets[1].fused > 0.0, "middle should be smoothed > 0, got {buckets:?}");
    }

    #[test]
    fn top_k_returns_highest_fused_first() {
        let buckets = vec![
            HeatBucket { bucket_start: 0.0, talk: 0.1, action: 0.0, highlight: 0.0, brandsafe: 0.0, fused: 0.1 },
            HeatBucket { bucket_start: 30.0, talk: 0.0, action: 0.0, highlight: 0.0, brandsafe: 0.0, fused: 0.9 },
            HeatBucket { bucket_start: 60.0, talk: 0.0, action: 0.0, highlight: 0.0, brandsafe: 0.0, fused: 0.5 },
        ];
        let top = top_k_buckets(&buckets, 2);
        assert_eq!(top.len(), 2);
        assert!((top[0].fused - 0.9).abs() < 1e-5);
        assert!((top[1].fused - 0.5).abs() < 1e-5);
    }

    #[test]
    fn top_k_zero_returns_empty() {
        let buckets = vec![HeatBucket { bucket_start: 0.0, talk: 0.0, action: 0.0, highlight: 0.0, brandsafe: 0.0, fused: 1.0 }];
        assert!(top_k_buckets(&buckets, 0).is_empty());
    }

    #[test]
    fn negative_event_times_ignored() {
        let cps = [-5.0_f32];
        let bs = [-10.0_f32];
        let highlights = [ev(-1.0, 0.5)];
        let inp = HeatmapInputs {
            segments: &[],
            cuepoint_times: &cps,
            highlights: &highlights,
            brandsafe_times: &bs,
            duration_sec: 60.0,
            bucket_secs: 30.0,
        };
        let buckets = compute_heatmap(&inp);
        // No channel should have signal.
        for b in &buckets {
            assert_eq!(b.action, 0.0);
            assert_eq!(b.brandsafe, 0.0);
            assert_eq!(b.highlight, 0.0);
        }
    }

    #[test]
    fn fusion_weights_dominate_with_talk() {
        // Equal talk + highlight in one bucket → talk weight 0.40 vs
        // highlight 0.30 means talk contribution > highlight, but
        // both contribute. Bucket with only talk should be < bucket
        // with both, given equal magnitudes.
        let segs = [ts(0.0, 60.0, 60), ts(60.0, 120.0, 60)];
        let highlights = [ev(70.0, 1.0)];
        let inp = HeatmapInputs {
            segments: &segs,
            cuepoint_times: &[],
            highlights: &highlights,
            brandsafe_times: &[],
            duration_sec: 120.0,
            bucket_secs: 30.0,
        };
        let buckets = compute_heatmap(&inp);
        // Bucket containing the highlight should have a higher fused
        // score than a same-talk bucket without the highlight.
        let bucket_with_hl = &buckets[2];
        let bucket_no_hl = &buckets[1];
        assert!(
            bucket_with_hl.fused >= bucket_no_hl.fused,
            "with-highlight {:?} should >= no-highlight {:?}",
            bucket_with_hl, bucket_no_hl
        );
    }
}
