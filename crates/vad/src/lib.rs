//! Voice activity detection / noise gate — DAW Strip Silence for streams.
//!
//! Real DAWs ship a noise gate (Pro Tools Strip Silence, Logic Noise
//! Gate) that opens when audio rises above a threshold and closes when
//! it falls below — with hysteresis so micro-fluctuations don't chatter
//! the gate. This crate ports that idea to PVR post-production: take
//! an ffmpeg-derived RMS envelope, return the intervals where the user
//! is speaking and a list of inter-speech gaps the editor can ripple-
//! delete to auto-tighten the recording.
//!
//! Complements [`strivo-deadair`] (which only finds silence above a
//! single threshold) with proper hysteresis + attack/release timing —
//! short blips above the floor don't open the gate, and a brief breath
//! between words doesn't close it.
//!
//! Pure data, no IO. Twelve tests cover hysteresis, attack/release
//! timing, single-frame extremes, JSON roundtrip, and the
//! tightening-recommendations rollup.

use serde::{Deserialize, Serialize};

/// One frame-level RMS measurement (same shape strivo-beat-detect uses,
/// kept local so this crate doesn't pull a circular dep).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeFrame {
    pub time_sec: f32,
    pub rms_db: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceInterval {
    pub start_sec: f32,
    pub end_sec: f32,
    /// Mean RMS dB across the open interval — helps the SPA colour-code
    /// loud vs whispered runs.
    pub mean_db: f32,
}

impl VoiceInterval {
    pub fn duration(&self) -> f32 {
        (self.end_sec - self.start_sec).max(0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SilenceGap {
    pub start_sec: f32,
    pub end_sec: f32,
}

impl SilenceGap {
    pub fn duration(&self) -> f32 {
        (self.end_sec - self.start_sec).max(0.0)
    }
}

/// DAW-style gate tunables. Defaults work for typical podcast / stream
/// commentary recordings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GateKnobs {
    /// Frame must clear this dB level to start opening the gate.
    pub open_db: f32,
    /// Frame must fall below this to start closing. Hysteresis = open
    /// is above close so short blips don't chatter the gate.
    pub close_db: f32,
    /// Minimum contiguous duration above `open_db` before the gate is
    /// considered open. Filters single-frame spikes.
    pub min_open_sec: f32,
    /// Minimum contiguous duration below `close_db` before the gate is
    /// closed. A breath between words shorter than this stays "open".
    pub min_close_sec: f32,
    /// Pre-roll / post-roll padding around each detected interval — the
    /// gate widens by this amount so a real attack at the start of a
    /// word isn't clipped. Default 75 ms.
    pub pad_sec: f32,
}

impl Default for GateKnobs {
    fn default() -> Self {
        Self {
            open_db: -30.0,
            close_db: -38.0,
            min_open_sec: 0.05,
            min_close_sec: 0.4,
            pad_sec: 0.075,
        }
    }
}

/// Run the gate. Returns the closed list of intervals where audio is
/// "voice-on". Empty input returns empty.
pub fn detect_voice(samples: &[EnvelopeFrame], knobs: &GateKnobs) -> Vec<VoiceInterval> {
    if samples.is_empty() {
        return vec![];
    }
    // State machine: Closed → Opening (timer counting toward min_open) →
    //                 Open → Closing (timer counting toward min_close) →
    //                 Closed.
    #[derive(Debug)]
    enum State {
        Closed,
        Opening { since: f32 },
        Open,
        Closing { since: f32 },
    }
    let mut state = State::Closed;
    let mut current_start: Option<f32> = None;
    let mut current_sum = 0.0f32;
    let mut current_count = 0u32;
    let mut out: Vec<VoiceInterval> = Vec::new();

    for f in samples {
        match &state {
            State::Closed => {
                if f.rms_db >= knobs.open_db {
                    state = State::Opening { since: f.time_sec };
                }
            }
            State::Opening { since } => {
                if f.rms_db >= knobs.open_db {
                    if f.time_sec - since >= knobs.min_open_sec {
                        // Gate opens. Backdate the interval start by
                        // the pre-roll pad, clamped to >= 0.
                        let start = (since - knobs.pad_sec).max(0.0);
                        current_start = Some(start);
                        current_sum = f.rms_db;
                        current_count = 1;
                        state = State::Open;
                    }
                } else {
                    // Fell below threshold before min_open — back to closed.
                    state = State::Closed;
                }
            }
            State::Open => {
                current_sum += f.rms_db;
                current_count += 1;
                if f.rms_db < knobs.close_db {
                    state = State::Closing { since: f.time_sec };
                }
            }
            State::Closing { since } => {
                // Don't pollute the mean with the closing tail; once we
                // start closing the samples are below close_db and would
                // drag the mean toward the noise floor.
                if f.rms_db < knobs.close_db {
                    if f.time_sec - since >= knobs.min_close_sec {
                        if let Some(start) = current_start.take() {
                            let mean = current_sum / current_count.max(1) as f32;
                            // Post-roll pad on the trailing edge.
                            let end = since + knobs.pad_sec;
                            out.push(VoiceInterval {
                                start_sec: start,
                                end_sec: end,
                                mean_db: mean,
                            });
                        }
                        current_sum = 0.0;
                        current_count = 0;
                        state = State::Closed;
                    }
                } else {
                    state = State::Open;
                }
            }
        }
    }
    // Flush any open interval at end-of-stream.
    if let Some(start) = current_start {
        if current_count > 0 {
            let mean = current_sum / current_count as f32;
            let end = samples.last().unwrap().time_sec;
            out.push(VoiceInterval {
                start_sec: start,
                end_sec: end + knobs.pad_sec,
                mean_db: mean,
            });
        }
    }
    out
}

/// Roll up the gaps between voice intervals as ripple-delete
/// recommendations. The editor's auto-tighten button applies these
/// directly.
///
/// `min_keep_sec` shields short gaps (natural beats / breaths) from
/// being trimmed. `total_duration_sec` is the recording length so the
/// tail gap after the last voice run also surfaces.
pub fn tightening_recommendations(
    intervals: &[VoiceInterval],
    total_duration_sec: f32,
    min_keep_sec: f32,
) -> Vec<SilenceGap> {
    let mut gaps = Vec::new();
    let mut cursor = 0.0f32;
    for iv in intervals {
        let gap = iv.start_sec - cursor;
        if gap > min_keep_sec {
            gaps.push(SilenceGap {
                start_sec: cursor,
                end_sec: iv.start_sec,
            });
        }
        cursor = iv.end_sec;
    }
    if total_duration_sec > cursor {
        let gap = total_duration_sec - cursor;
        if gap > min_keep_sec {
            gaps.push(SilenceGap {
                start_sec: cursor,
                end_sec: total_duration_sec,
            });
        }
    }
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t: f32, db: f32) -> EnvelopeFrame {
        EnvelopeFrame { time_sec: t, rms_db: db }
    }

    /// Build a synthetic envelope: alternating speech / silence blocks.
    /// `blocks` is a list of (duration_sec, rms_db).
    fn synth(blocks: &[(f32, f32)], frame_dt: f32) -> Vec<EnvelopeFrame> {
        let mut out = Vec::new();
        let mut t = 0.0f32;
        for (dur, db) in blocks {
            let n = (dur / frame_dt).round() as usize;
            for _ in 0..n {
                out.push(frame(t, *db));
                t += frame_dt;
            }
        }
        out
    }

    #[test]
    fn empty_envelope_yields_no_intervals() {
        let out = detect_voice(&[], &GateKnobs::default());
        assert!(out.is_empty());
    }

    #[test]
    fn flat_silence_yields_no_intervals() {
        let env = synth(&[(5.0, -50.0)], 0.05);
        let out = detect_voice(&env, &GateKnobs::default());
        assert!(out.is_empty());
    }

    #[test]
    fn steady_speech_block_opens_once_closes_once() {
        // 0..1s silent, 1..3s speech, 3..5s silent.
        let env = synth(&[(1.0, -50.0), (2.0, -20.0), (2.0, -50.0)], 0.05);
        let out = detect_voice(&env, &GateKnobs::default());
        assert_eq!(out.len(), 1);
        let iv = &out[0];
        // Pre-roll pads the start back to ~0.925; post-roll extends end.
        assert!(iv.start_sec < 1.0 && iv.start_sec >= 0.9, "start={}", iv.start_sec);
        assert!(iv.end_sec > 3.0 && iv.end_sec <= 3.5, "end={}", iv.end_sec);
        assert!(iv.mean_db < -15.0 && iv.mean_db > -25.0);
    }

    #[test]
    fn brief_spike_below_min_open_does_not_open_gate() {
        // A single 25ms spike at -20 dB amid silence — below min_open=50ms.
        let mut env = synth(&[(2.0, -50.0)], 0.05);
        env[20].rms_db = -20.0; // single frame at -20
        let out = detect_voice(&env, &GateKnobs::default());
        assert!(out.is_empty());
    }

    #[test]
    fn breath_between_words_keeps_gate_open() {
        // 2s speech · 0.2s breath (silence, < min_close=400ms) · 2s speech.
        let env = synth(&[
            (1.0, -50.0),
            (2.0, -20.0),
            (0.2, -50.0),
            (2.0, -20.0),
            (1.0, -50.0),
        ], 0.05);
        let out = detect_voice(&env, &GateKnobs::default());
        // Should collapse to ONE long interval, not two.
        assert_eq!(out.len(), 1);
        let iv = &out[0];
        assert!(iv.duration() > 4.0);
    }

    #[test]
    fn long_pause_closes_gate_into_two_intervals() {
        // 2s speech · 0.6s silence (> min_close=400ms) · 2s speech.
        let env = synth(&[
            (1.0, -50.0),
            (2.0, -20.0),
            (0.6, -50.0),
            (2.0, -20.0),
            (1.0, -50.0),
        ], 0.05);
        let out = detect_voice(&env, &GateKnobs::default());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn hysteresis_band_resists_chatter() {
        // Audio sits at -33 dB (between close=-38 and open=-30). Should
        // NEVER trigger a fresh open since it never rises above -30.
        let env = synth(&[(5.0, -33.0)], 0.05);
        let out = detect_voice(&env, &GateKnobs::default());
        assert!(out.is_empty());
    }

    #[test]
    fn pad_widens_interval_edges_by_pre_and_post_roll() {
        let mut knobs = GateKnobs::default();
        knobs.pad_sec = 0.2;
        let env = synth(&[(1.0, -50.0), (2.0, -20.0), (2.0, -50.0)], 0.05);
        let out = detect_voice(&env, &knobs);
        assert_eq!(out.len(), 1);
        let iv = &out[0];
        // Speech ran 1.0..3.0; with 0.2 pad the interval should be ~0.8..3.2.
        assert!((iv.start_sec - 0.8).abs() < 0.1, "start={}", iv.start_sec);
        assert!((iv.end_sec - 3.2).abs() < 0.15, "end={}", iv.end_sec);
    }

    #[test]
    fn pad_does_not_underflow_below_zero() {
        // Speech starts immediately at t=0 → pre-roll pad would go
        // negative; should clamp to 0.
        let env = synth(&[(2.0, -20.0), (2.0, -50.0)], 0.05);
        let out = detect_voice(&env, &GateKnobs::default());
        assert!(!out.is_empty());
        assert_eq!(out[0].start_sec, 0.0);
    }

    #[test]
    fn tightening_recommendations_rolls_up_gaps_between_intervals() {
        let intervals = vec![
            VoiceInterval { start_sec: 5.0,  end_sec: 10.0, mean_db: -20.0 },
            VoiceInterval { start_sec: 15.0, end_sec: 25.0, mean_db: -20.0 },
        ];
        let gaps = tightening_recommendations(&intervals, 30.0, 1.0);
        // 3 gaps: 0..5, 10..15, 25..30.
        assert_eq!(gaps.len(), 3);
        assert_eq!(gaps[0].start_sec, 0.0);
        assert_eq!(gaps[0].end_sec, 5.0);
        assert_eq!(gaps[1].start_sec, 10.0);
        assert_eq!(gaps[1].end_sec, 15.0);
        assert_eq!(gaps[2].start_sec, 25.0);
        assert_eq!(gaps[2].end_sec, 30.0);
    }

    #[test]
    fn tightening_recommendations_skips_short_gaps() {
        let intervals = vec![
            VoiceInterval { start_sec: 5.0, end_sec: 10.0, mean_db: -20.0 },
            VoiceInterval { start_sec: 10.5, end_sec: 15.0, mean_db: -20.0 },
        ];
        // min_keep=1.0 — the 0.5s gap between intervals is preserved.
        let gaps = tightening_recommendations(&intervals, 20.0, 1.0);
        // 2 gaps: 0..5 and 15..20; the 0.5s middle gap is kept.
        assert_eq!(gaps.len(), 2);
        assert_eq!(gaps[0].end_sec, 5.0);
        assert_eq!(gaps[1].start_sec, 15.0);
    }

    #[test]
    fn json_roundtrip_preserves_voice_interval() {
        let iv = VoiceInterval {
            start_sec: 1.0,
            end_sec: 3.5,
            mean_db: -17.2,
        };
        let s = serde_json::to_string(&iv).unwrap();
        let back: VoiceInterval = serde_json::from_str(&s).unwrap();
        assert!((back.mean_db - -17.2).abs() < 1e-3);
        assert_eq!(back.duration(), 2.5);
    }
}
