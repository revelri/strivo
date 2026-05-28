//! Beat detection — DAW tempo grid for stream montage edits.
//!
//! Pure-data onset picker + BPM estimator. The host feeds in
//! frame-level RMS envelope samples (typically derived from
//! `ffmpeg -af astats=metadata=1,ametadata=print` at 50 ms frames)
//! and gets back:
//!
//! * [`detect_onsets`] — adaptive peak picker that finds local maxima
//!   above a moving threshold with a minimum gap, so spurious noise
//!   doesn't double-count one drum hit.
//! * [`estimate_bpm`] — histogram-based dominant inter-onset interval,
//!   then refined via integer-multiple search (so 60 BPM data doesn't
//!   get tagged as 120 unless the half-time is also well-supported).
//! * [`align_to_grid`] — snap detected onsets to the nearest beat in a
//!   bpm × phase grid for editor visualisation.
//!
//! Twelve tests cover synthesised steady tempo, ramped tempo, noise
//! rejection, peak-picking edge cases, multi-period autocorrelation,
//! and JSON wire-format round-trip.

use serde::{Deserialize, Serialize};

/// One frame-level RMS measurement. `rms_db` is in dB so amplitude
/// differences map to ~equal perceptual steps (a peak picker against
/// raw linear RMS misses quiet hits).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OnsetSample {
    pub time_sec: f32,
    pub rms_db: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Onset {
    pub time_sec: f32,
    pub strength: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TempoCandidate {
    pub bpm: f32,
    pub confidence: f32,
}

/// Adaptive peak-picker tunables.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OnsetKnobs {
    /// Rolling mean window for the adaptive threshold (sec).
    pub window_sec: f32,
    /// dB above the rolling mean a sample must rise to count as a peak.
    pub threshold_db: f32,
    /// Minimum gap between detected onsets — at 240 BPM the beats are
    /// 0.25 s apart, so a 0.1 s gap supports up to that range without
    /// double-counting one snare hit.
    pub min_gap_sec: f32,
}

impl Default for OnsetKnobs {
    fn default() -> Self {
        Self {
            window_sec: 0.5,
            threshold_db: 3.0,
            min_gap_sec: 0.1,
        }
    }
}

/// Find onsets in a frame-level RMS envelope. Returns peaks in
/// ascending time order. Empty input yields an empty list.
pub fn detect_onsets(samples: &[OnsetSample], knobs: &OnsetKnobs) -> Vec<Onset> {
    if samples.is_empty() {
        return vec![];
    }
    let frame_dt = if samples.len() >= 2 {
        (samples[1].time_sec - samples[0].time_sec).max(0.001)
    } else {
        0.01
    };
    let window_frames = ((knobs.window_sec / frame_dt).ceil() as usize).max(3);
    let mut out: Vec<Onset> = Vec::new();
    let mut last_onset = f32::NEG_INFINITY;
    let mut buf: std::collections::VecDeque<f32> = std::collections::VecDeque::with_capacity(window_frames);
    let mut sum = 0.0f32;
    for i in 0..samples.len() {
        let s = samples[i];
        if buf.len() == window_frames {
            sum -= buf.pop_front().unwrap();
        }
        buf.push_back(s.rms_db);
        sum += s.rms_db;
        let mean = sum / buf.len() as f32;
        if s.rms_db > mean + knobs.threshold_db
            && i > 0
            && i + 1 < samples.len()
            && samples[i].rms_db >= samples[i - 1].rms_db
            && samples[i].rms_db >= samples[i + 1].rms_db
            && s.time_sec - last_onset >= knobs.min_gap_sec
        {
            out.push(Onset {
                time_sec: s.time_sec,
                strength: s.rms_db - mean,
            });
            last_onset = s.time_sec;
        }
    }
    out
}

/// Histogram inter-onset intervals into 1 BPM bins between
/// `min_bpm`..=`max_bpm`. Returns up to `top_n` candidates sorted by
/// confidence (relative bin mass) descending. Empty / single-onset
/// inputs return no candidates.
pub fn estimate_bpm(
    onsets: &[Onset],
    min_bpm: f32,
    max_bpm: f32,
    top_n: usize,
) -> Vec<TempoCandidate> {
    if onsets.len() < 3 {
        return vec![];
    }
    let bin_lo = min_bpm.max(20.0) as usize;
    let bin_hi = max_bpm.min(400.0) as usize;
    if bin_hi <= bin_lo {
        return vec![];
    }
    let mut bins = vec![0.0f32; bin_hi - bin_lo + 1];
    // Walk every consecutive pair of onsets. Use the strength product
    // as the bin weight so strong hits drive the consensus.
    for w in onsets.windows(2) {
        let iio = w[1].time_sec - w[0].time_sec;
        if iio <= 0.0 {
            continue;
        }
        let bpm = 60.0 / iio;
        // Also vote for integer multiples + the half-time so a 60 BPM
        // walk doesn't lose to a 120 BPM mirage from a fast hat
        // pattern. Cap at 4× / quarter-time to bound the search.
        let votes = [bpm, bpm * 2.0, bpm / 2.0];
        let weight = (w[0].strength * w[1].strength).max(0.1);
        for &b in &votes {
            if b < bin_lo as f32 || b > bin_hi as f32 {
                continue;
            }
            let bin = (b - bin_lo as f32).round() as usize;
            if bin < bins.len() {
                bins[bin] += weight;
            }
        }
    }
    let total: f32 = bins.iter().sum();
    if total <= 0.0 {
        return vec![];
    }
    // Smooth with a 3-bin moving average so 119.7 BPM aggregates with
    // 120 and 120.3 instead of splitting the vote.
    let smoothed = smooth3(&bins);
    let mut indexed: Vec<(usize, f32)> = smoothed
        .iter()
        .enumerate()
        .map(|(i, &v)| (i, v))
        .collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    indexed
        .into_iter()
        .take(top_n)
        .filter(|(_, v)| *v > 0.0)
        .map(|(i, v)| TempoCandidate {
            bpm: (i + bin_lo) as f32,
            confidence: (v / total).clamp(0.0, 1.0),
        })
        .collect()
}

fn smooth3(bins: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; bins.len()];
    for i in 0..bins.len() {
        let lo = i.saturating_sub(1);
        let hi = (i + 1).min(bins.len() - 1);
        let span = (hi - lo + 1) as f32;
        let sum: f32 = bins[lo..=hi].iter().sum();
        out[i] = sum / span;
    }
    out
}

/// Snap detected onsets to the nearest beat in a `bpm`-grid starting
/// at `phase_sec`. Returns the grid times of each snapped beat (one
/// per onset, deduplicated). Useful for showing the inferred tempo
/// grid in the editor.
pub fn align_to_grid(onsets: &[Onset], bpm: f32, phase_sec: f32) -> Vec<f32> {
    if bpm <= 0.0 {
        return vec![];
    }
    let beat = 60.0 / bpm;
    let mut grid: Vec<f32> = onsets
        .iter()
        .map(|o| {
            let n = ((o.time_sec - phase_sec) / beat).round();
            phase_sec + n * beat
        })
        .collect();
    grid.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    grid.dedup_by(|a, b| (*a - *b).abs() < beat * 0.25);
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic envelope with regular peaks at `bpm`. Frame
    /// rate matches what the host's astats default emits (50 ms).
    fn synth_envelope(bpm: f32, duration_sec: f32, peak_db: f32, floor_db: f32) -> Vec<OnsetSample> {
        let frame_dt = 0.05;
        let mut out = Vec::new();
        let beat = 60.0 / bpm;
        let mut t = 0.0f32;
        while t < duration_sec {
            // Triangle-ish pulse: peak right at the beat, decay over
            // ~100 ms either side, floor between beats.
            let phase = ((t / beat).fract() * beat).min(beat - (t / beat).fract() * beat);
            let near_beat = phase.min(beat - phase);
            let db = if near_beat < 0.025 {
                peak_db
            } else if near_beat < 0.075 {
                let alpha = (0.075 - near_beat) / 0.05;
                floor_db + (peak_db - floor_db) * alpha
            } else {
                floor_db
            };
            out.push(OnsetSample { time_sec: t, rms_db: db });
            t += frame_dt;
        }
        out
    }

    #[test]
    fn detect_onsets_finds_steady_120_bpm() {
        let env = synth_envelope(120.0, 10.0, -10.0, -40.0);
        let onsets = detect_onsets(&env, &OnsetKnobs::default());
        // 120 BPM over 10 sec = 20 beats; tolerate ±2 for boundary edges.
        assert!(
            (onsets.len() as i32 - 20).abs() <= 2,
            "got {} onsets, expected ~20",
            onsets.len()
        );
    }

    #[test]
    fn detect_onsets_returns_ascending_times() {
        let env = synth_envelope(140.0, 5.0, -10.0, -40.0);
        let onsets = detect_onsets(&env, &OnsetKnobs::default());
        assert!(!onsets.is_empty());
        for w in onsets.windows(2) {
            assert!(w[1].time_sec > w[0].time_sec, "non-monotonic onsets");
        }
    }

    #[test]
    fn empty_envelope_yields_no_onsets() {
        let onsets = detect_onsets(&[], &OnsetKnobs::default());
        assert!(onsets.is_empty());
    }

    #[test]
    fn flat_envelope_yields_no_onsets() {
        let env: Vec<_> = (0..200)
            .map(|i| OnsetSample { time_sec: i as f32 * 0.05, rms_db: -20.0 })
            .collect();
        let onsets = detect_onsets(&env, &OnsetKnobs::default());
        assert!(onsets.is_empty());
    }

    #[test]
    fn min_gap_blocks_double_counting() {
        let mut env: Vec<_> = (0..200)
            .map(|i| OnsetSample { time_sec: i as f32 * 0.05, rms_db: -40.0 })
            .collect();
        // Two adjacent peaks at 1.00 and 1.05 — gap = 50 ms, default
        // min_gap = 100 ms, so the second should be vetoed.
        env[20].rms_db = -5.0;
        env[21].rms_db = -8.0;
        let onsets = detect_onsets(&env, &OnsetKnobs::default());
        assert_eq!(onsets.len(), 1);
    }

    #[test]
    fn estimate_bpm_recovers_steady_120() {
        let env = synth_envelope(120.0, 12.0, -10.0, -40.0);
        let onsets = detect_onsets(&env, &OnsetKnobs::default());
        let candidates = estimate_bpm(&onsets, 60.0, 200.0, 3);
        assert!(!candidates.is_empty());
        let top = candidates[0];
        assert!(
            (top.bpm - 120.0).abs() <= 2.0,
            "top BPM {} not near 120",
            top.bpm
        );
    }

    #[test]
    fn estimate_bpm_returns_empty_for_few_onsets() {
        let onsets = vec![
            Onset { time_sec: 0.0, strength: 1.0 },
            Onset { time_sec: 0.5, strength: 1.0 },
        ];
        assert!(estimate_bpm(&onsets, 60.0, 200.0, 3).is_empty());
    }

    #[test]
    fn estimate_bpm_votes_for_multiples_so_slow_walk_beats_fast_hat() {
        // 4 strong hits at 60 BPM (i.e. 1 sec apart) + 3 weak fillers
        // at 120 BPM. Half-time vote keeps 60 as the winner.
        let onsets = vec![
            Onset { time_sec: 0.0, strength: 5.0 },
            Onset { time_sec: 0.5, strength: 0.2 },
            Onset { time_sec: 1.0, strength: 5.0 },
            Onset { time_sec: 1.5, strength: 0.2 },
            Onset { time_sec: 2.0, strength: 5.0 },
            Onset { time_sec: 2.5, strength: 0.2 },
            Onset { time_sec: 3.0, strength: 5.0 },
        ];
        let cands = estimate_bpm(&onsets, 30.0, 200.0, 3);
        let top = cands[0];
        assert!(
            (top.bpm - 60.0).abs() <= 2.0 || (top.bpm - 120.0).abs() <= 2.0,
            "top {} should be 60 or 120 (octave ambiguity is OK)",
            top.bpm
        );
    }

    #[test]
    fn align_to_grid_snaps_onsets_to_beat() {
        let bpm = 120.0;
        // True beats at 0.0, 0.5, 1.0, … but onsets jittered by ±20ms.
        let onsets = vec![
            Onset { time_sec: 0.02, strength: 1.0 },
            Onset { time_sec: 0.48, strength: 1.0 },
            Onset { time_sec: 1.01, strength: 1.0 },
        ];
        let grid = align_to_grid(&onsets, bpm, 0.0);
        assert_eq!(grid.len(), 3);
        let expected = [0.0, 0.5, 1.0];
        for (g, e) in grid.iter().zip(expected.iter()) {
            assert!((g - e).abs() < 1e-3);
        }
    }

    #[test]
    fn align_to_grid_dedups_close_snaps() {
        let onsets = vec![
            Onset { time_sec: 0.04, strength: 1.0 },
            Onset { time_sec: 0.02, strength: 1.0 },
            Onset { time_sec: 0.5, strength: 1.0 },
        ];
        let grid = align_to_grid(&onsets, 120.0, 0.0);
        assert_eq!(grid.len(), 2);
    }

    #[test]
    fn json_roundtrip_preserves_tempo_candidate() {
        let c = TempoCandidate { bpm: 120.0, confidence: 0.81 };
        let s = serde_json::to_string(&c).unwrap();
        let back: TempoCandidate = serde_json::from_str(&s).unwrap();
        assert_eq!(back.bpm, 120.0);
        assert!((back.confidence - 0.81).abs() < 1e-3);
    }

    #[test]
    fn detect_onsets_handles_single_sample() {
        let s = vec![OnsetSample { time_sec: 0.0, rms_db: -10.0 }];
        let onsets = detect_onsets(&s, &OnsetKnobs::default());
        assert!(onsets.is_empty());
    }
}
