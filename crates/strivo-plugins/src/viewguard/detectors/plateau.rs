//! PlateauVariance — flags suspiciously low viewer variance.
//!
//! Real viewer counts breathe: people leave for snacks, mobile reconnects
//! cycle, the platform-side viewer estimator rounds differently each
//! refresh. Empirically the coefficient of variation (σ/μ) of a 10-min
//! window of viewer samples sits around 0.02–0.08 for healthy streams.
//! Bot fleets show as flatlines: CV approaches zero across the entire
//! session.
//!
//! Score: 1 - CV / 0.02 over a 20-bin (10-min) window, evaluated as the
//! min across all available 20-bin windows once the stream has run long
//! enough.

use serde_json::json;

use super::{Detector, DetectorKind, SignalScore};
use crate::viewguard::stats::ChannelStats;

#[derive(Default)]
pub struct PlateauVariance;

const WINDOW: usize = 20; // 10 min
const MIN_SAMPLES: usize = 60; // 30 min
/// CV below this is suspicious. Real streams sit ~0.02–0.08.
const CV_FLOOR: f32 = 0.02;

impl Detector for PlateauVariance {
    fn kind(&self) -> DetectorKind {
        DetectorKind::PlateauVariance
    }

    fn evaluate(&self, stats: &ChannelStats) -> Option<SignalScore> {
        let vals = stats.values();
        if vals.len() < MIN_SAMPLES {
            return None;
        }

        let mut min_cv = f32::INFINITY;
        let mut min_window_mean = 0.0_f32;
        for w in vals.windows(WINDOW) {
            let mean = w.iter().map(|&v| v as f64).sum::<f64>() / w.len() as f64;
            if mean < 20.0 {
                // tiny streams have huge integer-quantization CV; skip
                continue;
            }
            let var = w
                .iter()
                .map(|&v| {
                    let d = v as f64 - mean;
                    d * d
                })
                .sum::<f64>()
                / w.len() as f64;
            let cv = (var.sqrt() / mean) as f32;
            if cv < min_cv {
                min_cv = cv;
                min_window_mean = mean as f32;
            }
        }
        if !min_cv.is_finite() {
            return None;
        }

        // Map CV into a 0..1 anomaly score: CV at the floor → 1, at
        // 2x floor → 0.5, ≥ 4x floor → 0.
        let score = (1.0 - (min_cv / CV_FLOOR - 1.0).max(0.0)).clamp(0.0, 1.0);
        if score < 0.2 {
            return None;
        }

        let confidence = ((vals.len() as f32) / (4.0 * 120.0)).clamp(0.25, 1.0);

        Some(SignalScore {
            kind: DetectorKind::PlateauVariance,
            score,
            confidence,
            evidence: json!({
                "min_cv": min_cv,
                "window_mean": min_window_mean,
                "window_bins": WINDOW,
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
    fn flat_plateau_fires() {
        // Perfectly flat 500 viewers for 90 bins.
        let s = stats_from(&vec![500; 90]);
        let r = PlateauVariance.evaluate(&s).expect("flatline should fire");
        assert!(r.score > 0.8, "perfect flatline should score near 1, got {}", r.score);
    }

    #[test]
    fn natural_breathing_does_not_fire() {
        // CV ~0.05 — well above floor. Alternating 480/520 around 500.
        let mut vals = Vec::new();
        for i in 0..120 {
            let v = if i % 2 == 0 { 480 } else { 520 };
            vals.push(v);
        }
        let s = stats_from(&vals);
        let r = PlateauVariance.evaluate(&s);
        assert!(r.is_none(), "natural noise should not fire, got {r:?}");
    }

    #[test]
    fn tiny_stream_skipped() {
        // mean < 20 — should not fire even when flat.
        let s = stats_from(&vec![5; 90]);
        let r = PlateauVariance.evaluate(&s);
        assert!(r.is_none());
    }
}
