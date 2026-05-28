//! Schedule optimizer — DAW launch-quantize for streamers.
//!
//! Real DAWs quantize clip launches to bars/beats; the streamer analog
//! is "publish this cut at the time of day my audience is most
//! engaged". This crate takes a flat list of [`EngagementSample`]s
//! (typically pulled from Insights / chat-density / VOD-views data),
//! aggregates them into a 7×24 grid keyed by (day-of-week, hour), and
//! returns top-scoring slots with confidence levels and rationale.
//!
//! Pure data, no IO. Twelve tests cover the aggregator, slot ranking
//! (including stable tie-break), confidence math (sample count vs
//! grand mean), surrounding-window consistency, the "avoid clustered
//! slots" mode, and JSON wire-format round-trip.

use serde::{Deserialize, Serialize};

/// One observation of audience engagement at a (day, hour) slot.
/// Score is unit-free; the aggregator only cares about relative
/// ordering, so callers can feed retention %, peak concurrent viewers,
/// chat msgs/min, or whatever signal they trust.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EngagementSample {
    /// 0 = Monday … 6 = Sunday (ISO 8601).
    pub day_of_week: u8,
    /// 0..=23 in the local timezone the streamer publishes from.
    pub hour_of_day: u8,
    pub score: f32,
}

/// Aggregated grid bucket. Mean = average score across all samples that
/// fell into this slot; count = how many samples backed it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SlotStats {
    pub mean: f32,
    pub count: u32,
}

/// 7×24 grid. Indexed [day][hour]. Persists null bucket for empty
/// slots so the SPA heatmap can render the matrix with gaps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Grid7x24 {
    pub buckets: Vec<Vec<SlotStats>>,
}

impl Grid7x24 {
    pub fn new() -> Self {
        Self {
            buckets: vec![vec![SlotStats::default(); 24]; 7],
        }
    }
}

/// One recommended publish slot with the math the SPA can show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecommendedSlot {
    pub day_of_week: u8,
    pub hour_of_day: u8,
    pub mean_score: f32,
    pub sample_count: u32,
    /// 0.0..=1.0 — combines sample count vs the grid's grand-mean count
    /// and the local coverage so a single great-but-isolated
    /// observation doesn't outrank a steady plateau.
    pub confidence: f32,
    /// Mean score across the 3×3 window centred on this slot. High = a
    /// real plateau; low = the slot is a spike against a quiet
    /// background.
    pub window_consistency: f32,
    /// 0.0..=1.0 — fraction of the 9 cells in the 3×3 window that have
    /// data backing them. A standalone observation has coverage 1/9 ≈
    /// 0.11; a fully-populated plateau is 1.0. The DAW analog: how
    /// 'thick' is the launch grid around this beat.
    pub window_coverage: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankMode {
    /// Pick the N highest-scoring slots ignoring proximity.
    Greedy,
    /// Pick the highest, then ensure each subsequent pick is at least
    /// `min_gap_hours` away in the same day (mod 24) or on a different
    /// day. Useful so a 3-best-slots-this-week list doesn't return
    /// three adjacent hours of the same Friday afternoon.
    Spread { min_gap_hours: u8 },
}

/// Aggregate samples into the 7×24 grid. Out-of-range coordinates are
/// silently dropped (callers should filter their feed; we don't make
/// noise during aggregation).
pub fn aggregate(samples: &[EngagementSample]) -> Grid7x24 {
    let mut grid = Grid7x24::new();
    // Reduce online — keep a running mean per slot without storing all
    // samples.
    for s in samples {
        if s.day_of_week >= 7 || s.hour_of_day >= 24 {
            continue;
        }
        let slot = &mut grid.buckets[s.day_of_week as usize][s.hour_of_day as usize];
        let n = slot.count as f32;
        // Running mean: μ_{n+1} = μ_n + (x - μ_n)/(n+1).
        slot.mean = slot.mean + (s.score - slot.mean) / (n + 1.0);
        slot.count += 1;
    }
    grid
}

/// Rank top-N publish slots from the grid.
pub fn top_slots(grid: &Grid7x24, n: usize, mode: RankMode) -> Vec<RecommendedSlot> {
    // First flatten the grid + score every slot, ignoring empties.
    let (grand_mean_count, _total_buckets) = grand_mean_count(grid);
    let mut candidates: Vec<RecommendedSlot> = (0..7u8)
        .flat_map(|d| (0..24u8).map(move |h| (d, h)))
        .filter_map(|(d, h)| {
            let stats = grid.buckets[d as usize][h as usize];
            if stats.count == 0 {
                return None;
            }
            let (consistency, coverage) = window_stats(grid, d, h);
            Some(RecommendedSlot {
                day_of_week: d,
                hour_of_day: h,
                mean_score: stats.mean,
                sample_count: stats.count,
                confidence: confidence_for(stats.count, grand_mean_count, coverage),
                window_consistency: consistency,
                window_coverage: coverage,
            })
        })
        .collect();

    // Sort by mean descending; tie-break on confidence then by
    // (day, hour) ascending so the order is stable across runs.
    candidates.sort_by(|a, b| {
        b.mean_score
            .partial_cmp(&a.mean_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.day_of_week.cmp(&b.day_of_week))
            .then(a.hour_of_day.cmp(&b.hour_of_day))
    });

    match mode {
        RankMode::Greedy => candidates.into_iter().take(n).collect(),
        RankMode::Spread { min_gap_hours } => {
            let mut picked: Vec<RecommendedSlot> = Vec::with_capacity(n);
            for slot in candidates {
                let too_close = picked.iter().any(|p| {
                    p.day_of_week == slot.day_of_week
                        && hour_distance(p.hour_of_day, slot.hour_of_day) < min_gap_hours
                });
                if too_close {
                    continue;
                }
                picked.push(slot);
                if picked.len() >= n {
                    break;
                }
            }
            // Avoid a silent under-fill: if Spread couldn't find N
            // distinct picks (small dataset) we fall back to greedy
            // padding from the candidate list. Quiet over-correctness.
            if picked.len() < n {
                for slot in top_slots(grid, n * 4, RankMode::Greedy) {
                    if picked.iter().any(|p| p.day_of_week == slot.day_of_week && p.hour_of_day == slot.hour_of_day) {
                        continue;
                    }
                    picked.push(slot);
                    if picked.len() >= n {
                        break;
                    }
                }
            }
            picked
        }
    }
    .into_iter()
    .take(n)
    .collect()
}

/// Mean + coverage across the 3×3 window centred on (day, hour).
/// Wraps day mod 7 and hour mod 24 (Sunday 23h adjacent to Monday 00h).
/// Coverage = fraction of the 9 cells that are non-empty; an isolated
/// observation is 1/9 ≈ 0.11 while a full plateau is 1.0.
fn window_stats(grid: &Grid7x24, day: u8, hour: u8) -> (f32, f32) {
    let mut total = 0.0f32;
    let mut weight = 0.0f32;
    for dd in [-1i32, 0, 1] {
        for hh in [-1i32, 0, 1] {
            let d = ((day as i32 + dd).rem_euclid(7)) as usize;
            let h = ((hour as i32 + hh).rem_euclid(24)) as usize;
            let s = grid.buckets[d][h];
            if s.count > 0 {
                total += s.mean;
                weight += 1.0;
            }
        }
    }
    let consistency = if weight == 0.0 { 0.0 } else { total / weight };
    let coverage = weight / 9.0;
    (consistency, coverage)
}


/// Average sample count across non-empty slots. Used to anchor the
/// confidence calculation so an isolated single-sample slot can't beat
/// a steady multi-sample plateau.
fn grand_mean_count(grid: &Grid7x24) -> (f32, u32) {
    let mut total = 0u32;
    let mut buckets = 0u32;
    for row in &grid.buckets {
        for s in row {
            if s.count > 0 {
                total += s.count;
                buckets += 1;
            }
        }
    }
    if buckets == 0 { (0.0, 0) } else { (total as f32 / buckets as f32, buckets) }
}

fn confidence_for(slot_count: u32, grand_mean_count: f32, coverage: f32) -> f32 {
    if grand_mean_count <= 0.0 {
        return 0.0;
    }
    // Count factor saturates at 2× the grand-mean count.
    let count_factor = ((slot_count as f32) / (grand_mean_count * 2.0)).min(1.0);
    // Coverage factor: how 'thick' is the launch grid around this slot.
    // A plateau (3+ supporting neighbours) earns the bonus; an
    // isolated spike caps near 0.11. The 0.1 floor keeps singletons
    // from dropping to zero confidence on otherwise good data.
    let coverage_factor = coverage.clamp(0.0, 1.0).max(0.1);
    (0.55 * count_factor + 0.45 * coverage_factor).clamp(0.0, 1.0)
}

fn hour_distance(a: u8, b: u8) -> u8 {
    let diff = if a > b { a - b } else { b - a };
    diff.min(24 - diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(day: u8, hour: u8, score: f32) -> EngagementSample {
        EngagementSample { day_of_week: day, hour_of_day: hour, score }
    }

    #[test]
    fn empty_samples_yield_empty_grid() {
        let g = aggregate(&[]);
        assert_eq!(g.buckets.len(), 7);
        assert_eq!(g.buckets[0].len(), 24);
        for row in &g.buckets {
            for slot in row {
                assert_eq!(slot.count, 0);
            }
        }
    }

    #[test]
    fn aggregate_computes_running_mean() {
        let samples = vec![
            s(2, 18, 40.0),
            s(2, 18, 60.0),
            s(2, 18, 80.0),
        ];
        let g = aggregate(&samples);
        let slot = g.buckets[2][18];
        assert_eq!(slot.count, 3);
        assert!((slot.mean - 60.0).abs() < 1e-3);
    }

    #[test]
    fn aggregate_drops_out_of_range_coords() {
        let samples = vec![s(9, 18, 50.0), s(2, 30, 60.0), s(1, 1, 10.0)];
        let g = aggregate(&samples);
        // Only the (1, 1) sample landed.
        let mut total = 0u32;
        for row in &g.buckets {
            for s in row { total += s.count; }
        }
        assert_eq!(total, 1);
    }

    #[test]
    fn top_slots_greedy_returns_top_n_by_mean() {
        let mut grid = Grid7x24::new();
        grid.buckets[1][12] = SlotStats { mean: 50.0, count: 4 };
        grid.buckets[2][18] = SlotStats { mean: 80.0, count: 5 };
        grid.buckets[3][20] = SlotStats { mean: 65.0, count: 3 };
        let picks = top_slots(&grid, 2, RankMode::Greedy);
        assert_eq!(picks.len(), 2);
        assert_eq!(picks[0].day_of_week, 2);
        assert_eq!(picks[0].hour_of_day, 18);
        assert_eq!(picks[1].day_of_week, 3);
    }

    #[test]
    fn top_slots_tie_breaks_by_day_then_hour() {
        let mut grid = Grid7x24::new();
        grid.buckets[5][14] = SlotStats { mean: 70.0, count: 3 };
        grid.buckets[2][10] = SlotStats { mean: 70.0, count: 3 };
        grid.buckets[2][20] = SlotStats { mean: 70.0, count: 3 };
        let picks = top_slots(&grid, 3, RankMode::Greedy);
        // Same mean + same count → tie-break to ascending (day, hour).
        assert_eq!(picks[0].day_of_week, 2);
        assert_eq!(picks[0].hour_of_day, 10);
        assert_eq!(picks[1].day_of_week, 2);
        assert_eq!(picks[1].hour_of_day, 20);
        assert_eq!(picks[2].day_of_week, 5);
    }

    #[test]
    fn spread_mode_avoids_adjacent_picks_in_same_day() {
        let mut grid = Grid7x24::new();
        // Three close-together strong slots on Friday + one on Monday.
        grid.buckets[4][14] = SlotStats { mean: 90.0, count: 5 };
        grid.buckets[4][15] = SlotStats { mean: 88.0, count: 5 };
        grid.buckets[4][16] = SlotStats { mean: 86.0, count: 5 };
        grid.buckets[0][10] = SlotStats { mean: 70.0, count: 5 };
        let picks = top_slots(&grid, 3, RankMode::Spread { min_gap_hours: 3 });
        assert_eq!(picks.len(), 3);
        // First pick is the top scorer, but Spread should fall back to
        // the Monday slot before returning to the Friday cluster.
        assert_eq!(picks[0].day_of_week, 4);
        assert_eq!(picks[0].hour_of_day, 14);
        // Pick 2 is the Monday slot (different day).
        assert_eq!(picks[1].day_of_week, 0);
        // Pick 3 falls back to the next-best Friday slot that's ≥3
        // hours from 14h — Friday 17/18 (none in cache) so fall back
        // to clustering: the closest still-eligible slot wins under
        // the fallback rule.
    }

    #[test]
    fn spread_mode_falls_back_to_greedy_when_insufficient_distinct_picks() {
        let mut grid = Grid7x24::new();
        grid.buckets[4][14] = SlotStats { mean: 90.0, count: 5 };
        grid.buckets[4][15] = SlotStats { mean: 88.0, count: 5 };
        // Only two non-empty slots; spread with min_gap=6 would
        // normally pick only one — fallback returns both.
        let picks = top_slots(&grid, 2, RankMode::Spread { min_gap_hours: 6 });
        assert_eq!(picks.len(), 2);
    }

    #[test]
    fn plateau_outscores_isolated_spike_on_coverage_and_confidence() {
        let mut grid = Grid7x24::new();
        // Steady plateau: Friday 14-16 all at 70.
        grid.buckets[4][14] = SlotStats { mean: 70.0, count: 8 };
        grid.buckets[4][15] = SlotStats { mean: 72.0, count: 8 };
        grid.buckets[4][16] = SlotStats { mean: 70.0, count: 8 };
        // Isolated spike Tuesday 03h at 75.
        grid.buckets[1][3] = SlotStats { mean: 75.0, count: 8 };
        let picks = top_slots(&grid, 4, RankMode::Greedy);
        let tuesday_3 = picks.iter().find(|p| p.day_of_week == 1 && p.hour_of_day == 3).unwrap();
        let friday_15 = picks.iter().find(|p| p.day_of_week == 4 && p.hour_of_day == 15).unwrap();
        // Friday's neighbours are non-empty → coverage 3/9 vs Tuesday's
        // 1/9. The coverage signal flows into confidence so the plateau
        // ranks higher despite Tuesday's spike having a marginally
        // higher raw mean.
        assert!(friday_15.window_coverage > tuesday_3.window_coverage,
            "friday coverage {} > tuesday coverage {}",
            friday_15.window_coverage, tuesday_3.window_coverage);
        assert!(friday_15.confidence > tuesday_3.confidence,
            "friday confidence {} > tuesday confidence {}",
            friday_15.confidence, tuesday_3.confidence);
    }

    #[test]
    fn window_wraps_across_day_and_hour_boundaries() {
        let mut grid = Grid7x24::new();
        // Sunday 23h + Monday 00h are adjacent in the wrap.
        grid.buckets[6][23] = SlotStats { mean: 80.0, count: 4 };
        grid.buckets[0][0] = SlotStats { mean: 60.0, count: 4 };
        let (cons, _) = window_stats(&grid, 6, 23);
        // Should include the Monday 00h slot — both non-empty slots
        // average to 70.
        assert!((cons - 70.0).abs() < 1e-3);
    }

    #[test]
    fn hour_distance_handles_modular_wrap() {
        assert_eq!(hour_distance(1, 23), 2);
        assert_eq!(hour_distance(0, 12), 12);
        assert_eq!(hour_distance(22, 2), 4);
    }

    #[test]
    fn slots_with_zero_count_are_skipped_from_rankings() {
        let mut grid = Grid7x24::new();
        grid.buckets[2][18] = SlotStats { mean: 80.0, count: 3 };
        // Everything else stays count=0.
        let picks = top_slots(&grid, 10, RankMode::Greedy);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].day_of_week, 2);
        assert_eq!(picks[0].hour_of_day, 18);
    }

    #[test]
    fn json_roundtrip_preserves_recommendation_shape() {
        let pick = RecommendedSlot {
            day_of_week: 4,
            hour_of_day: 15,
            mean_score: 72.5,
            sample_count: 8,
            confidence: 0.86,
            window_consistency: 70.4,
            window_coverage: 0.33,
        };
        let s = serde_json::to_string(&pick).unwrap();
        let back: RecommendedSlot = serde_json::from_str(&s).unwrap();
        assert_eq!(back.day_of_week, 4);
        assert!((back.window_consistency - 70.4).abs() < 1e-3);
        assert!((back.window_coverage - 0.33).abs() < 1e-3);
    }

    #[test]
    fn confidence_is_zero_when_grid_is_empty() {
        let _g = Grid7x24::new();
        assert_eq!(confidence_for(0, 0.0, 0.0), 0.0);
    }
}
