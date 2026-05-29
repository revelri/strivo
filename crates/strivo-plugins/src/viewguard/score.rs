//! Score aggregator + band classification.
//!
//! Combines per-detector [`SignalScore`]s into a single 0..1 final
//! suspicion score, applying gating to reduce false positives:
//!
//! - Single-detector firings are downgraded — even a perfect-looking
//!   PlateauVariance hit alone is "watch", not "suspect", because raids
//!   and donations-pause-mode can look flat for a window.
//! - Two or more independent detectors firing above their confidence
//!   floor count fully — this is the FP gate.

use serde::Serialize;

use super::detectors::SignalScore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Band {
    Clean,
    Watch,
    Suspect,
    Fraudulent,
}

impl Band {
    pub fn from_score(s: f32) -> Self {
        if s >= 0.80 { Band::Fraudulent }
        else if s >= 0.50 { Band::Suspect }
        else if s >= 0.25 { Band::Watch }
        else { Band::Clean }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Band::Clean => "clean",
            Band::Watch => "watch",
            Band::Suspect => "suspect",
            Band::Fraudulent => "fraudulent",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AggregatedVerdict {
    pub final_score: f32,
    pub band: Band,
    pub contributors: Vec<Contributor>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contributor {
    pub detector: &'static str,
    pub score: f32,
    pub weight: f32,
    pub confidence: f32,
}

/// Confidence floor below which a detector is recorded but not counted
/// toward the gating quorum.
const CONFIDENCE_FLOOR: f32 = 0.4;

pub fn aggregate(signals: &[SignalScore]) -> AggregatedVerdict {
    if signals.is_empty() {
        return AggregatedVerdict {
            final_score: 0.0,
            band: Band::Clean,
            contributors: Vec::new(),
        };
    }

    let mut contributors: Vec<Contributor> = signals
        .iter()
        .map(|s| Contributor {
            detector: s.kind.name(),
            score: s.score,
            weight: s.kind.weight(),
            confidence: s.confidence,
        })
        .collect();
    contributors.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let qualified: Vec<&Contributor> = contributors
        .iter()
        .filter(|c| c.confidence >= CONFIDENCE_FLOOR)
        .collect();

    let raw_weighted: f32 = contributors
        .iter()
        .map(|c| c.score * c.weight * c.confidence)
        .sum::<f32>()
        / contributors
            .iter()
            .map(|c| c.weight * c.confidence)
            .sum::<f32>()
            .max(0.01);

    // Gating: if fewer than 2 qualified detectors, cap the final score
    // at the top of the "watch" band so single-signal hits never read
    // as suspect.
    let final_score = if qualified.len() < 2 {
        raw_weighted.min(0.49)
    } else {
        raw_weighted
    };

    AggregatedVerdict {
        final_score,
        band: Band::from_score(final_score),
        contributors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::detectors::DetectorKind;
    use serde_json::json;

    fn sig(kind: DetectorKind, score: f32, confidence: f32) -> SignalScore {
        SignalScore { kind, score, confidence, evidence: json!({}) }
    }

    #[test]
    fn empty_is_clean() {
        let v = aggregate(&[]);
        assert_eq!(v.band, Band::Clean);
    }

    #[test]
    fn single_high_signal_capped_at_watch() {
        let v = aggregate(&[sig(DetectorKind::PlateauVariance, 1.0, 1.0)]);
        assert!(v.final_score <= 0.49, "got {}", v.final_score);
        assert!(matches!(v.band, Band::Watch | Band::Clean));
    }

    #[test]
    fn two_strong_signals_reach_suspect() {
        let v = aggregate(&[
            sig(DetectorKind::PlateauVariance, 0.8, 1.0),
            sig(DetectorKind::SpikeShape, 0.7, 1.0),
        ]);
        assert!(v.final_score >= 0.5, "got {}", v.final_score);
        assert!(matches!(v.band, Band::Suspect | Band::Fraudulent));
    }

    #[test]
    fn low_confidence_does_not_gate() {
        // Two signals, both below confidence floor — should still be capped.
        let v = aggregate(&[
            sig(DetectorKind::PlateauVariance, 0.9, 0.2),
            sig(DetectorKind::SpikeShape, 0.9, 0.2),
        ]);
        assert!(v.final_score <= 0.49);
    }

    #[test]
    fn band_thresholds() {
        assert_eq!(Band::from_score(0.0), Band::Clean);
        assert_eq!(Band::from_score(0.24), Band::Clean);
        assert_eq!(Band::from_score(0.30), Band::Watch);
        assert_eq!(Band::from_score(0.60), Band::Suspect);
        assert_eq!(Band::from_score(0.85), Band::Fraudulent);
    }
}
