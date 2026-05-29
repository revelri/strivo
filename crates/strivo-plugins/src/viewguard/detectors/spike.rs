//! SpikeShape — flags step-function jumps in viewer count.
//!
//! Real audiences ramp; paid bot fleets arrive in one tick. We compare
//! the largest single-bin delta in the recent window against the
//! channel's own historical noise floor (median absolute deviation of
//! per-bin deltas over the full ring). A delta > k·MAD with the prior
//! viewer level low enough that the jump represents a meaningful
//! multiplier fires.
//!
//! Score: clamp(jump_z / 12, 0, 1). Confidence ramps with sample count.

use serde_json::json;

use super::{Detector, DetectorKind, SignalScore};
use crate::viewguard::stats::ChannelStats;

#[derive(Default)]
pub struct SpikeShape;

/// Need at least this many samples to compute a meaningful baseline.
const MIN_SAMPLES: usize = 60; // 30 min @ 30s
/// Look-back window for the "recent" max delta.
const RECENT_WINDOW: usize = 10; // last 5 min
/// Discontinuity floor — ignore tiny jumps even when MAD is near zero.
const MIN_ABS_JUMP: u32 = 50;

impl Detector for SpikeShape {
    fn kind(&self) -> DetectorKind {
        DetectorKind::SpikeShape
    }

    fn evaluate(&self, stats: &ChannelStats) -> Option<SignalScore> {
        let vals = stats.values();
        if vals.len() < MIN_SAMPLES {
            return None;
        }

        // Per-bin deltas (signed).
        let deltas: Vec<i64> = vals
            .windows(2)
            .map(|w| w[1] as i64 - w[0] as i64)
            .collect();
        if deltas.is_empty() {
            return None;
        }

        // Baseline noise = MAD of deltas over the full window.
        let mut abs: Vec<i64> = deltas.iter().map(|d| d.abs()).collect();
        abs.sort_unstable();
        let mad = abs[abs.len() / 2].max(1) as f32;

        // Recent largest absolute delta + the viewer level just before.
        let recent_start = deltas.len().saturating_sub(RECENT_WINDOW);
        let (rel_idx, &recent_max) = deltas[recent_start..]
            .iter()
            .enumerate()
            .max_by_key(|(_, d)| d.abs())
            .unwrap();
        let abs_max = recent_max.abs() as u32;
        if abs_max < MIN_ABS_JUMP {
            return None;
        }

        // Index in vals where the jump landed.
        let jump_idx = recent_start + rel_idx + 1;
        let prior = vals[jump_idx - 1].max(1) as f32;
        let multiplier = vals[jump_idx] as f32 / prior;

        // Z-like score: how many MADs is the jump? Then divide by 12
        // (arrived-at empirically: 12 MAD ≈ "obvious step function").
        let z = abs_max as f32 / mad;
        let raw = (z / 12.0).clamp(0.0, 1.0);

        // Only count it as a viewbot if the *multiplier* is also high —
        // a 5000-viewer stream gaining 500 is normal, gaining 5000
        // instantly is not. This is the FP control on legitimate raids:
        // raids ramp over ~30-60s, paid bots land in one bin.
        let multi_gain = (multiplier - 1.0).max(0.0);
        let multi_factor = (multi_gain / 1.5).clamp(0.0, 1.0); // 2.5x jump = full weight
        let score = raw * multi_factor;
        if score < 0.15 {
            return None;
        }

        // Confidence: full at 4h of data, half at 1h.
        let confidence = ((vals.len() as f32) / (4.0 * 120.0)).clamp(0.25, 1.0);

        Some(SignalScore {
            kind: DetectorKind::SpikeShape,
            score,
            confidence,
            evidence: json!({
                "z": z,
                "mad": mad,
                "abs_delta": abs_max,
                "multiplier": multiplier,
                "prior_viewers": vals[jump_idx - 1],
                "after_viewers": vals[jump_idx],
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewguard::stats::{BIN_SECS, ChannelStats};
    use chrono::{DateTime, Utc};

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(s, 0).unwrap()
    }

    fn stats_from(vs: &[u32]) -> ChannelStats {
        let mut s = ChannelStats::new("c".into(), "twitch".into(), "C".into());
        for (i, v) in vs.iter().enumerate() {
            s.push(t(i as i64 * BIN_SECS), *v);
        }
        s
    }

    #[test]
    fn organic_ramp_does_not_fire() {
        // 60 bins of slow ramp 100→700: legitimate going-viral.
        let vals: Vec<u32> = (0..60).map(|i| 100 + i * 10).collect();
        let s = stats_from(&vals);
        let r = SpikeShape.evaluate(&s);
        assert!(r.is_none(), "organic ramp should not fire, got {r:?}");
    }

    #[test]
    fn step_function_fires() {
        // 80 bins flat at 100, then jump to 5000.
        let mut vals: Vec<u32> = vec![100; 80];
        vals.extend(vec![5000; 10]);
        let s = stats_from(&vals);
        let r = SpikeShape.evaluate(&s).expect("step should fire");
        assert!(r.score > 0.3, "step should score high, got {}", r.score);
    }

    #[test]
    fn insufficient_samples_returns_none() {
        let s = stats_from(&[100; 10]);
        assert!(SpikeShape.evaluate(&s).is_none());
    }
}
