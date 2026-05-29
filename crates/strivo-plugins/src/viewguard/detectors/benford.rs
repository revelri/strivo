//! BenfordDigits — leading-digit distribution of viewer samples.
//!
//! Real-world counts (populations, financial figures, viewer counts on
//! organic streams) follow Benford's law: leading-digit 1 appears in
//! ~30% of samples, 9 in ~5%. Hand-rolled bot services often emit
//! suspiciously round or uniform-distributed numbers — their digit
//! histogram diverges from Benford.
//!
//! This is a *weak* signal by design (weight 0.4 in the aggregator).
//! On its own it's not actionable; combined with SpikeShape or
//! PlateauVariance it tightens confidence.
//!
//! We use chi-squared distance to the expected Benford distribution,
//! normalized to 0..1.

use serde_json::json;

use super::{Detector, DetectorKind, SignalScore};
use crate::viewguard::stats::ChannelStats;

#[derive(Default)]
pub struct BenfordDigits;

const MIN_SAMPLES: usize = 200; // ~100 min @ 30s — Benford needs a lot

/// Benford reference distribution for leading digit 1..9.
const BENFORD: [f32; 9] = [
    0.301, 0.176, 0.125, 0.097, 0.079, 0.067, 0.058, 0.051, 0.046,
];

impl Detector for BenfordDigits {
    fn kind(&self) -> DetectorKind {
        DetectorKind::BenfordDigits
    }

    fn evaluate(&self, stats: &ChannelStats) -> Option<SignalScore> {
        let vals = stats.values();
        if vals.len() < MIN_SAMPLES {
            return None;
        }
        // Skip viewer counts < 10 (no meaningful leading digit).
        let mut hist = [0u32; 9];
        let mut n = 0u32;
        for v in vals.iter().filter(|&&v| v >= 10) {
            let d = leading_digit(*v);
            if (1..=9).contains(&d) {
                hist[d as usize - 1] += 1;
                n += 1;
            }
        }
        if n < MIN_SAMPLES as u32 / 2 {
            return None;
        }
        // Chi-squared distance.
        let mut chi2 = 0.0_f32;
        for i in 0..9 {
            let observed = hist[i] as f32 / n as f32;
            let expected = BENFORD[i];
            let diff = observed - expected;
            chi2 += (diff * diff) / expected;
        }
        // Empirical: chi2 of 0.1 ≈ noticeable divergence, 0.3 ≈ blatant.
        let score = (chi2 / 0.3).clamp(0.0, 1.0);
        if score < 0.3 {
            return None;
        }
        let confidence = ((n as f32) / 600.0).clamp(0.25, 0.8);

        Some(SignalScore {
            kind: DetectorKind::BenfordDigits,
            score,
            confidence,
            evidence: json!({
                "chi2": chi2,
                "n": n,
                "hist": hist,
            }),
        })
    }
}

fn leading_digit(mut v: u32) -> u32 {
    while v >= 10 {
        v /= 10;
    }
    v
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
    fn leading_digit_basic() {
        assert_eq!(leading_digit(1), 1);
        assert_eq!(leading_digit(42), 4);
        assert_eq!(leading_digit(9999), 9);
        assert_eq!(leading_digit(100), 1);
    }

    #[test]
    fn uniform_round_numbers_fire() {
        // Bot service emits exactly 5000 viewers forever — leading digit
        // always 5 → enormous chi2 vs Benford.
        let s = stats_from(&vec![5000; 300]);
        let r = BenfordDigits.evaluate(&s).expect("uniform should fire");
        assert!(r.score > 0.8, "got {}", r.score);
    }

    #[test]
    fn benford_distributed_does_not_fire() {
        // Mix of values roughly following Benford: many 1xxx, fewer 9xxx.
        let mut vs = Vec::new();
        for _ in 0..90 { vs.push(1234); }
        for _ in 0..50 { vs.push(2345); }
        for _ in 0..38 { vs.push(3456); }
        for _ in 0..30 { vs.push(4567); }
        for _ in 0..24 { vs.push(5678); }
        for _ in 0..20 { vs.push(6789); }
        for _ in 0..17 { vs.push(7890); }
        for _ in 0..16 { vs.push(8901); }
        for _ in 0..15 { vs.push(9012); }
        // duplicates to hit MIN_SAMPLES
        vs.extend(vs.clone());
        let s = stats_from(&vs);
        let r = BenfordDigits.evaluate(&s);
        assert!(r.is_none(), "Benford-shaped should not fire, got {r:?}");
    }
}
