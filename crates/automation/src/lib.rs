//! Volume automation — DAW clip/track automation for streams.
//!
//! Every DAW lets you draw time-keyed gain curves over a track; streamers
//! need the same for ducking game audio while talking, ramping intro
//! music, and fading outros. This crate models the points + emits the
//! ffmpeg invocation that bakes them at render time.
//!
//! Wire shape: a sorted list of [`AutomationPoint`]s (time + gain in dB +
//! curve mode). The renderer:
//!
//! 1. Pre-samples between points at a fixed grid (default 50 ms) to
//!    approximate non-step curves — ffmpeg's `volume` filter only
//!    accepts discrete `sendcmd` steps.
//! 2. Emits the `asendcmd=…` block as `t volume volume <linear>|…`.
//!
//! Pure data, no IO. Twelve tests cover the curve interpolators, the
//! dB↔linear conversion, the asendcmd serialiser, and out-of-range
//! sampling.

use serde::{Deserialize, Serialize};

/// Interpolation shape between two adjacent automation points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// Hold the previous value until the next point's time.
    Step,
    /// Straight-line ramp in dB space.
    Linear,
    /// Smoothstep using a half-cosine — the classic DAW "S-curve".
    Cosine,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub time_sec: f32,
    pub gain_db: f32,
    /// Curve to use when interpolating FROM this point to the next.
    #[serde(default = "default_curve")]
    pub curve: Curve,
}

fn default_curve() -> Curve { Curve::Linear }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VolumeAutomation {
    pub points: Vec<AutomationPoint>,
}

impl VolumeAutomation {
    /// Sorted-by-time view of the points (defensive copy). Adjacent
    /// duplicate times collapse to the later entry — the SPA editor
    /// occasionally emits both.
    pub fn sorted(&self) -> Vec<AutomationPoint> {
        let mut v = self.points.clone();
        v.sort_by(|a, b| a.time_sec.partial_cmp(&b.time_sec).unwrap_or(std::cmp::Ordering::Equal));
        // Walk left→right; when two consecutive points share a time (within
        // 1 ms), keep the LATER entry. Std's `dedup_by` keeps the first;
        // the editor UX is "the user moved this point, the prior value
        // should not survive" so later-wins is the useful semantic.
        let mut out: Vec<AutomationPoint> = Vec::with_capacity(v.len());
        for p in v {
            if let Some(last) = out.last_mut() {
                if (last.time_sec - p.time_sec).abs() < 1e-3 {
                    *last = p;
                    continue;
                }
            }
            out.push(p);
        }
        out
    }

    /// Evaluate the curve at `t_sec`. Before the first point we hold the
    /// first gain; after the last we hold the last. Empty automation
    /// returns 0 dB (unity gain).
    pub fn sample(&self, t_sec: f32) -> f32 {
        let pts = self.sorted();
        if pts.is_empty() {
            return 0.0;
        }
        if t_sec <= pts[0].time_sec {
            return pts[0].gain_db;
        }
        if let Some(last) = pts.last() {
            if t_sec >= last.time_sec {
                return last.gain_db;
            }
        }
        // Find the segment containing t.
        for w in pts.windows(2) {
            let a = &w[0];
            let b = &w[1];
            if t_sec >= a.time_sec && t_sec <= b.time_sec {
                let span = (b.time_sec - a.time_sec).max(1e-6);
                let u = ((t_sec - a.time_sec) / span).clamp(0.0, 1.0);
                return interpolate(a.gain_db, b.gain_db, u, a.curve);
            }
        }
        // Unreachable in practice.
        pts.last().unwrap().gain_db
    }

    /// Build the `asendcmd=` argument for the ffmpeg `volume` filter.
    /// Pre-samples at `step_secs` granularity inside each non-Step
    /// segment so the discrete command list approximates the curve.
    pub fn to_asendcmd(&self, step_secs: f32) -> String {
        let pts = self.sorted();
        if pts.is_empty() {
            return String::new();
        }
        let step = step_secs.max(0.005);
        let mut cmds: Vec<String> = Vec::new();
        // Initial setting at the first point's time.
        cmds.push(fmt_cmd(pts[0].time_sec, db_to_linear(pts[0].gain_db)));
        for w in pts.windows(2) {
            let a = &w[0];
            let b = &w[1];
            match a.curve {
                Curve::Step => {
                    cmds.push(fmt_cmd(b.time_sec, db_to_linear(b.gain_db)));
                }
                Curve::Linear | Curve::Cosine => {
                    let span = (b.time_sec - a.time_sec).max(0.0);
                    if span <= step {
                        cmds.push(fmt_cmd(b.time_sec, db_to_linear(b.gain_db)));
                        continue;
                    }
                    let n = (span / step).ceil() as usize;
                    for i in 1..=n {
                        let t = a.time_sec + (i as f32 * step).min(span);
                        let u = ((t - a.time_sec) / span).clamp(0.0, 1.0);
                        let db = interpolate(a.gain_db, b.gain_db, u, a.curve);
                        cmds.push(fmt_cmd(t, db_to_linear(db)));
                    }
                }
            }
        }
        format!("asendcmd=c='{}'", cmds.join("|"))
    }

    /// Build the full ffmpeg audio filter graph: a passthrough `volume`
    /// filter driven by the asendcmd block. The caller wires this into
    /// the audio-only filter chain on the editor render path.
    pub fn build_audio_filter(&self, step_secs: f32) -> String {
        let sendcmd = self.to_asendcmd(step_secs);
        if sendcmd.is_empty() {
            return "anull".to_string();
        }
        format!("{sendcmd},volume=1.0:eval=frame")
    }
}

fn interpolate(a_db: f32, b_db: f32, u: f32, curve: Curve) -> f32 {
    match curve {
        Curve::Step => a_db,
        Curve::Linear => a_db + (b_db - a_db) * u,
        Curve::Cosine => {
            // Half-cosine smoothstep — classic DAW fade-in / fade-out shape.
            let weight = 0.5 - 0.5 * (u * std::f32::consts::PI).cos();
            a_db + (b_db - a_db) * weight
        }
    }
}

/// Convert decibels to a linear gain multiplier. -inf dB → 0 by clamp.
/// Capped at -120 dB to avoid float underflow on very long automations.
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -120.0 { return 0.0; }
    10f32.powf(db / 20.0)
}

/// Inverse of [`db_to_linear`].
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 { return -120.0; }
    20.0 * linear.log10()
}

fn fmt_cmd(t: f32, linear: f32) -> String {
    format!("{:.3} volume volume {:.4}", t, linear)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(t: f32, db: f32, curve: Curve) -> AutomationPoint {
        AutomationPoint { time_sec: t, gain_db: db, curve }
    }

    #[test]
    fn empty_automation_samples_unity_gain() {
        let a = VolumeAutomation::default();
        assert_eq!(a.sample(0.0), 0.0);
        assert_eq!(a.sample(100.0), 0.0);
    }

    #[test]
    fn sample_clamps_before_first_and_after_last_point() {
        let a = VolumeAutomation {
            points: vec![pt(10.0, -6.0, Curve::Linear), pt(20.0, 0.0, Curve::Linear)],
        };
        assert_eq!(a.sample(0.0), -6.0);
        assert_eq!(a.sample(15.0), -3.0);
        assert_eq!(a.sample(25.0), 0.0);
    }

    #[test]
    fn linear_interpolation_is_straight_line_in_db() {
        let a = VolumeAutomation {
            points: vec![pt(0.0, -12.0, Curve::Linear), pt(4.0, 0.0, Curve::Linear)],
        };
        assert!((a.sample(2.0) - -6.0).abs() < 1e-3);
        assert!((a.sample(1.0) - -9.0).abs() < 1e-3);
    }

    #[test]
    fn step_curve_holds_until_next_point() {
        let a = VolumeAutomation {
            points: vec![pt(0.0, -6.0, Curve::Step), pt(5.0, 0.0, Curve::Linear)],
        };
        assert!((a.sample(2.5) - -6.0).abs() < 1e-3);
        assert!((a.sample(4.9) - -6.0).abs() < 1e-3);
        assert!((a.sample(5.0) - 0.0).abs() < 1e-3);
    }

    #[test]
    fn cosine_curve_is_smoothstep_shape() {
        let a = VolumeAutomation {
            points: vec![pt(0.0, 0.0, Curve::Cosine), pt(10.0, -12.0, Curve::Linear)],
        };
        let half = a.sample(5.0);
        // Midpoint should be exactly halfway in dB for symmetric cosine.
        assert!((half - -6.0).abs() < 1e-3);
        // Quarter-point of a cosine fade should be roughly 15% dimmer
        // than linear (-1.76 vs -3.0).
        let quarter = a.sample(2.5);
        assert!(quarter > -3.0, "cosine quarter {quarter} should be brighter than linear -3");
    }

    #[test]
    fn db_to_linear_handles_zero_and_negative_db() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_linear(-6.0) - 0.501187).abs() < 1e-3);
        assert!((db_to_linear(-20.0) - 0.1).abs() < 1e-6);
        assert_eq!(db_to_linear(-130.0), 0.0); // floor clamp
    }

    #[test]
    fn linear_to_db_inverts_db_to_linear() {
        for db in [-20.0, -12.0, -6.0, 0.0, 6.0] {
            let round = linear_to_db(db_to_linear(db));
            assert!((round - db).abs() < 1e-2, "roundtrip {db} → {round}");
        }
    }

    #[test]
    fn asendcmd_emits_initial_point_at_first_time() {
        let a = VolumeAutomation {
            points: vec![pt(2.0, 0.0, Curve::Linear), pt(10.0, -12.0, Curve::Linear)],
        };
        let cmd = a.to_asendcmd(1.0);
        assert!(cmd.starts_with("asendcmd=c='2.000 volume volume 1.0000"));
    }

    #[test]
    fn asendcmd_step_curve_skips_intermediate_samples() {
        let a = VolumeAutomation {
            points: vec![pt(0.0, -6.0, Curve::Step), pt(5.0, 0.0, Curve::Linear)],
        };
        let cmd = a.to_asendcmd(0.1);
        // Step curve → exactly two commands: 0.0 at -6 dB + 5.0 at 0 dB.
        let pipe_count = cmd.matches('|').count();
        assert_eq!(pipe_count, 1);
        assert!(cmd.contains("5.000 volume volume 1.0000"));
    }

    #[test]
    fn asendcmd_linear_curve_writes_dense_grid_at_step_resolution() {
        let a = VolumeAutomation {
            points: vec![pt(0.0, -12.0, Curve::Linear), pt(1.0, 0.0, Curve::Linear)],
        };
        let cmd = a.to_asendcmd(0.1);
        let count = cmd.matches('|').count() + 1; // commands = pipes + 1
        // 1 second / 100 ms = 10 steps, plus the initial → 11 commands.
        assert_eq!(count, 11);
    }

    #[test]
    fn build_audio_filter_returns_passthrough_for_empty_automation() {
        let a = VolumeAutomation::default();
        assert_eq!(a.build_audio_filter(0.05), "anull");
    }

    #[test]
    fn build_audio_filter_chains_volume_with_frame_eval() {
        let a = VolumeAutomation {
            points: vec![pt(0.0, 0.0, Curve::Linear), pt(2.0, -6.0, Curve::Linear)],
        };
        let f = a.build_audio_filter(0.5);
        assert!(f.contains("asendcmd=c='"));
        assert!(f.ends_with(",volume=1.0:eval=frame"));
    }

    #[test]
    fn duplicate_time_points_dedup_in_sorted_view() {
        let a = VolumeAutomation {
            points: vec![
                pt(0.0, -6.0, Curve::Linear),
                pt(0.0005, 0.0, Curve::Linear),
                pt(2.0, -12.0, Curve::Linear),
            ],
        };
        let v = a.sorted();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].gain_db, 0.0);
    }

    #[test]
    fn unsorted_points_are_sorted_on_sample() {
        let a = VolumeAutomation {
            points: vec![pt(10.0, 0.0, Curve::Linear), pt(0.0, -12.0, Curve::Linear)],
        };
        // Sample at the unsorted-first-element's time:
        assert!((a.sample(5.0) - -6.0).abs() < 1e-3);
    }
}
