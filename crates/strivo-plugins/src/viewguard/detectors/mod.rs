//! Statistical viewbot detectors.
//!
//! Each detector inspects a [`ChannelStats`] snapshot and optionally
//! emits a [`SignalScore`]. The aggregator in `score.rs` combines
//! independent firings using a gated weighted sum.

use serde::Serialize;

use crate::viewguard::stats::ChannelStats;

pub mod benford;
pub mod plateau;
pub mod spike;

/// Per-detector identifier (also persisted in `signals.detector`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DetectorKind {
    SpikeShape,
    PlateauVariance,
    BenfordDigits,
}

impl DetectorKind {
    pub fn name(&self) -> &'static str {
        match self {
            DetectorKind::SpikeShape => "spike_shape",
            DetectorKind::PlateauVariance => "plateau_variance",
            DetectorKind::BenfordDigits => "benford_digits",
        }
    }

    /// Aggregator weight. Higher = signal is more trusted (lower FP rate
    /// on legitimate streams). Benford is informational, weight is low.
    pub fn weight(&self) -> f32 {
        match self {
            DetectorKind::SpikeShape => 1.0,
            DetectorKind::PlateauVariance => 1.0,
            DetectorKind::BenfordDigits => 0.4,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalScore {
    pub kind: DetectorKind,
    /// 0..1 — how anomalous this looks.
    pub score: f32,
    /// 0..1 — how much we trust this score given sample count etc.
    pub confidence: f32,
    /// Free-form JSON evidence (detector-specific).
    pub evidence: serde_json::Value,
}

pub trait Detector {
    fn kind(&self) -> DetectorKind;
    fn evaluate(&self, stats: &ChannelStats) -> Option<SignalScore>;
}

/// Run every M1 detector and collect non-None results.
pub fn run_all(stats: &ChannelStats) -> Vec<SignalScore> {
    let detectors: [Box<dyn Detector>; 3] = [
        Box::new(spike::SpikeShape::default()),
        Box::new(plateau::PlateauVariance::default()),
        Box::new(benford::BenfordDigits::default()),
    ];
    detectors
        .iter()
        .filter_map(|d| d.evaluate(stats))
        .collect()
}
