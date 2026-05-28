//! strivo-insights-compare — stream comparison + retention proxy.
//!
//! Two pure-data primitives the SPA wires to the existing Crunchr /
//! Insights / Cuepoints surfaces:
//!
//!   1. [`compare_words`] — Jaccard overlap + symmetric difference of
//!      two top-K word lists. Used by the "compare two streams" UI to
//!      show "what's new / what's gone".
//!   2. [`compute_retention`] — bucket transcript activity + cuepoint
//!      density into a [0,1] per-bucket curve. Stands in for real
//!      audience-retention data (YouTube Analytics will replace it in
//!      iter 13's Heatmap plugin). The shape gives editors a usable
//!      "where did things slow down" view today.
//!
//! Zero IO. Caller passes in the lists they pulled from the database;
//! everything testable, fast, and free of fixture-file dependencies.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordCount {
    pub word: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordComparison {
    /// Words present in both lists, ordered by combined frequency.
    pub shared: Vec<SharedWord>,
    /// Words unique to A (sorted by count desc).
    pub only_a: Vec<WordCount>,
    /// Words unique to B (sorted by count desc).
    pub only_b: Vec<WordCount>,
    /// Jaccard similarity in [0, 1]. 1.0 = identical word sets.
    pub jaccard: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedWord {
    pub word: String,
    pub count_a: u64,
    pub count_b: u64,
    /// Multiplicative skew: how much more often this word appears in
    /// A than B. >1 = used more in A; <1 = used more in B.
    pub a_over_b: f32,
}

/// Compare two top-K word lists. Returns sets of shared / unique
/// entries with a Jaccard score. Case-insensitive matching.
pub fn compare_words(a: &[WordCount], b: &[WordCount]) -> WordComparison {
    let mut a_map: std::collections::HashMap<String, u64> = a
        .iter()
        .map(|w| (w.word.to_lowercase(), w.count))
        .collect();
    let mut b_map: std::collections::HashMap<String, u64> = b
        .iter()
        .map(|w| (w.word.to_lowercase(), w.count))
        .collect();

    let mut shared: Vec<SharedWord> = Vec::new();
    let shared_keys: Vec<String> = a_map.keys().filter(|k| b_map.contains_key(*k)).cloned().collect();
    for key in shared_keys {
        let count_a = a_map.remove(&key).unwrap_or(0);
        let count_b = b_map.remove(&key).unwrap_or(0);
        let a_over_b = if count_b == 0 {
            f32::INFINITY
        } else {
            count_a as f32 / count_b as f32
        };
        shared.push(SharedWord {
            word: key,
            count_a,
            count_b,
            a_over_b,
        });
    }
    // Sort shared by combined frequency desc — most-discussed words first.
    shared.sort_by(|x, y| (y.count_a + y.count_b).cmp(&(x.count_a + x.count_b)));

    let mut only_a: Vec<WordCount> = a_map
        .into_iter()
        .map(|(w, c)| WordCount { word: w, count: c })
        .collect();
    only_a.sort_by(|x, y| y.count.cmp(&x.count));
    let mut only_b: Vec<WordCount> = b_map
        .into_iter()
        .map(|(w, c)| WordCount { word: w, count: c })
        .collect();
    only_b.sort_by(|x, y| y.count.cmp(&x.count));

    let union_size = shared.len() + only_a.len() + only_b.len();
    let jaccard = if union_size == 0 {
        1.0
    } else {
        shared.len() as f32 / union_size as f32
    };

    WordComparison {
        shared,
        only_a,
        only_b,
        jaccard,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub start_sec: f32,
    pub end_sec: f32,
    pub word_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RetentionPoint {
    pub bucket_start: f32,
    pub talk_density: f32,
    pub action_density: f32,
    pub retention: f32,
}

/// Bucket the recording into windows of `bucket_secs`. For each
/// window:
///   - `talk_density` = words/second normalised against the loudest
///     bucket.
///   - `action_density` = cuepoints/second normalised against the
///     densest bucket.
///   - `retention` = 0.6 × talk + 0.4 × action, then smoothed with a
///     3-bucket moving average so a single quiet window doesn't tank
///     the curve.
///
/// Output is sorted chronologically and ready to render as a strip.
pub fn compute_retention(
    segments: &[Segment],
    cuepoints: &[f32],
    duration_sec: f32,
    bucket_secs: f32,
) -> Vec<RetentionPoint> {
    if duration_sec <= 0.0 || bucket_secs <= 0.0 {
        return Vec::new();
    }
    let n_buckets = (duration_sec / bucket_secs).ceil() as usize;
    if n_buckets == 0 {
        return Vec::new();
    }
    let mut talk = vec![0.0_f32; n_buckets];
    let mut action = vec![0.0_f32; n_buckets];

    // Talk: distribute each segment's word_count across the buckets
    // it spans, weighted by overlap (so a 20s segment spanning two
    // buckets contributes proportionally to each).
    for s in segments {
        if s.end_sec <= s.start_sec {
            continue;
        }
        let words_per_sec = (s.word_count as f32) / (s.end_sec - s.start_sec).max(0.001);
        let lo = (s.start_sec / bucket_secs).floor() as usize;
        let hi = ((s.end_sec / bucket_secs).ceil() as usize).min(n_buckets);
        for b in lo..hi {
            let b_lo = b as f32 * bucket_secs;
            let b_hi = b_lo + bucket_secs;
            let overlap = (s.end_sec.min(b_hi) - s.start_sec.max(b_lo)).max(0.0);
            if overlap > 0.0 {
                talk[b] += words_per_sec * overlap / bucket_secs;
            }
        }
    }
    for &cp in cuepoints {
        if cp < 0.0 {
            continue;
        }
        let b = (cp / bucket_secs).floor() as usize;
        if b < n_buckets {
            action[b] += 1.0;
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

    // Fuse + smooth (3-window moving average so single dead buckets
    // don't dominate the SPA strip).
    let mut points: Vec<RetentionPoint> = (0..n_buckets)
        .map(|i| RetentionPoint {
            bucket_start: i as f32 * bucket_secs,
            talk_density: talk[i],
            action_density: action[i],
            retention: 0.6 * talk[i] + 0.4 * action[i],
        })
        .collect();
    let raw: Vec<f32> = points.iter().map(|p| p.retention).collect();
    for i in 0..points.len() {
        let lo = i.saturating_sub(1);
        let hi = (i + 1).min(points.len() - 1);
        points[i].retention = (raw[lo] + raw[i] + raw[hi]) / 3.0;
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wc(word: &str, count: u64) -> WordCount {
        WordCount { word: word.to_string(), count }
    }

    #[test]
    fn compare_identical_lists_is_jaccard_one() {
        let a = vec![wc("alpha", 5), wc("beta", 3)];
        let b = vec![wc("alpha", 5), wc("beta", 3)];
        let r = compare_words(&a, &b);
        assert_eq!(r.jaccard, 1.0);
        assert_eq!(r.shared.len(), 2);
        assert!(r.only_a.is_empty());
        assert!(r.only_b.is_empty());
    }

    #[test]
    fn compare_disjoint_lists_is_jaccard_zero() {
        let a = vec![wc("alpha", 1)];
        let b = vec![wc("beta", 1)];
        let r = compare_words(&a, &b);
        assert_eq!(r.jaccard, 0.0);
        assert!(r.shared.is_empty());
    }

    #[test]
    fn compare_partial_overlap_jaccard() {
        let a = vec![wc("alpha", 4), wc("beta", 3), wc("gamma", 1)];
        let b = vec![wc("alpha", 5), wc("delta", 2)];
        let r = compare_words(&a, &b);
        // intersection {alpha} = 1; union {alpha, beta, gamma, delta} = 4 → 0.25.
        assert!((r.jaccard - 0.25).abs() < 1e-5);
        assert_eq!(r.shared.len(), 1);
        assert_eq!(r.shared[0].word, "alpha");
    }

    #[test]
    fn compare_case_insensitive() {
        let a = vec![wc("Alpha", 5)];
        let b = vec![wc("alpha", 3)];
        let r = compare_words(&a, &b);
        assert_eq!(r.shared.len(), 1);
        assert_eq!(r.shared[0].count_a, 5);
        assert_eq!(r.shared[0].count_b, 3);
    }

    #[test]
    fn compare_a_over_b_ratio() {
        let a = vec![wc("alpha", 10)];
        let b = vec![wc("alpha", 2)];
        let r = compare_words(&a, &b);
        assert_eq!(r.shared.len(), 1);
        assert!((r.shared[0].a_over_b - 5.0).abs() < 1e-5);
    }

    #[test]
    fn retention_empty_inputs_yield_empty() {
        let out = compute_retention(&[], &[], 0.0, 60.0);
        assert!(out.is_empty());
    }

    #[test]
    fn retention_distributes_words_across_buckets() {
        // Single 60s segment with 60 words straddling buckets 0 and 1.
        let segs = vec![Segment { start_sec: 30.0, end_sec: 90.0, word_count: 60 }];
        let out = compute_retention(&segs, &[], 120.0, 60.0);
        assert_eq!(out.len(), 2);
        // Both buckets should have non-zero talk density.
        assert!(out[0].talk_density > 0.0);
        assert!(out[1].talk_density > 0.0);
    }

    #[test]
    fn retention_action_density_counts_cuepoints() {
        let cps = vec![10.0_f32, 12.0, 65.0];
        let out = compute_retention(&[], &cps, 120.0, 60.0);
        assert_eq!(out.len(), 2);
        // Bucket 0 has 2 cuepoints, bucket 1 has 1.
        assert!(out[0].action_density > out[1].action_density);
    }

    #[test]
    fn retention_smoothing_pulls_neighbours() {
        // Three buckets with [1, 0, 1] talk pattern; smoothing
        // should pull the middle off zero.
        let segs = vec![
            Segment { start_sec: 0.0, end_sec: 60.0, word_count: 60 },
            Segment { start_sec: 120.0, end_sec: 180.0, word_count: 60 },
        ];
        let out = compute_retention(&segs, &[], 180.0, 60.0);
        assert_eq!(out.len(), 3);
        assert!(out[1].retention > 0.0, "middle should be smoothed > 0, got {out:?}");
    }

    #[test]
    fn retention_curve_normalised_in_unit_range() {
        let segs = (0..10)
            .map(|i| Segment {
                start_sec: (i * 60) as f32,
                end_sec: ((i + 1) * 60) as f32,
                word_count: 100 * (i as u32 + 1),
            })
            .collect::<Vec<_>>();
        let cps: Vec<f32> = (0..10).map(|i| (i * 60) as f32).collect();
        let out = compute_retention(&segs, &cps, 600.0, 60.0);
        for p in &out {
            assert!((0.0..=1.0).contains(&p.retention), "out of unit range: {p:?}");
        }
    }
}
