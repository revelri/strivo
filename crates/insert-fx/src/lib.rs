//! DAW-style insert effects chain.
//!
//! Every DAW lets you stack effects per track in order: voice gets
//! noise-reduction → de-esser → compressor → limiter; the game bus gets a
//! high-pass to clear room for the voice, then a soft limiter. StriVo's
//! equivalent is a per-recording ordered list of [`InsertEffect`] values; the
//! render path emits one ffmpeg filter per effect and stitches them with `,`.
//!
//! Pure-data: no IO here, no ffmpeg execution — just a typed model + a
//! string composer. The host wraps it; the host's render path is the only
//! place that actually invokes ffmpeg.
//!
//! Default presets ship for the two buses every streamer needs:
//! [`InsertChain::voice_bus_default`] and [`InsertChain::game_bus_default`].

use serde::{Deserialize, Serialize};

/// One effect in an insert chain.
///
/// Each variant carries its own params and maps to a single ffmpeg filter
/// invocation via [`InsertEffect::to_filter`]. The variants intentionally
/// mirror DAW plug-in names rather than ffmpeg filter names so the UI can
/// label them naturally — translation happens once, here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InsertEffect {
    /// High-pass / low-cut. `freq_hz` typical: 80 (voice), 120 (aggressive).
    HighPass { freq_hz: f64 },
    /// Low-pass / high-cut. `freq_hz` typical: 12000 (telephone), 16000 (air).
    LowPass { freq_hz: f64 },
    /// Single-band parametric EQ. `gain_db` ±18 typical; `width_q` 0.5..6.
    EqBand {
        freq_hz: f64,
        gain_db: f64,
        width_q: f64,
    },
    /// FFT-based broadband noise reduction. `amount` 0..1, default 0.5.
    NoiseReduction { amount: f64 },
    /// Sibilance reducer. `intensity` 0..1, `freq_hz` typical 6000.
    DeEsser { intensity: f64, freq_hz: f64 },
    /// Downward compressor. `ratio` ≥1; `threshold_db` ≤0; attack/release sec.
    Compressor {
        threshold_db: f64,
        ratio: f64,
        attack_sec: f64,
        release_sec: f64,
        makeup_db: f64,
    },
    /// Brick-wall limiter. `ceiling_db` typically -1.0 (broadcast-safe).
    Limiter { ceiling_db: f64, release_sec: f64 },
    /// Algorithmic reverb. `room_size` 0..1, `wet_db` ≤0.
    Reverb { room_size: f64, wet_db: f64 },
    /// Linear gain trim. `gain_db` ±24 sensible.
    Gain { gain_db: f64 },
}

impl InsertEffect {
    /// Emit the ffmpeg filter string for this effect.
    ///
    /// One filter per effect; never includes the `,` separator —
    /// [`InsertChain::to_filter`] glues them.
    pub fn to_filter(&self) -> String {
        match self {
            Self::HighPass { freq_hz } => format!("highpass=f={}", fmt_f(*freq_hz)),
            Self::LowPass { freq_hz } => format!("lowpass=f={}", fmt_f(*freq_hz)),
            Self::EqBand {
                freq_hz,
                gain_db,
                width_q,
            } => format!(
                "equalizer=f={}:t=q:w={}:g={}",
                fmt_f(*freq_hz),
                fmt_f(*width_q),
                fmt_f(*gain_db)
            ),
            // afftdn nr is in dB of noise floor reduction; map 0..1 → 6..30 dB.
            Self::NoiseReduction { amount } => {
                let nr_db = 6.0 + amount.clamp(0.0, 1.0) * 24.0;
                format!("afftdn=nr={}:nf=-25", fmt_f(nr_db))
            }
            // ffmpeg's deesser: i=intensity 0..1, f=freq normalised 0..1 (Nyquist).
            Self::DeEsser { intensity, freq_hz } => {
                let f_norm = (*freq_hz / 24000.0).clamp(0.0, 1.0);
                format!(
                    "deesser=i={}:f={}:m=0.5:s=o",
                    fmt_f(intensity.clamp(0.0, 1.0)),
                    fmt_f(f_norm)
                )
            }
            Self::Compressor {
                threshold_db,
                ratio,
                attack_sec,
                release_sec,
                makeup_db,
            } => format!(
                "acompressor=threshold={}dB:ratio={}:attack={}:release={}:makeup={}",
                fmt_f(*threshold_db),
                fmt_f(*ratio),
                fmt_f(attack_sec * 1000.0),
                fmt_f(release_sec * 1000.0),
                fmt_f(*makeup_db),
            ),
            Self::Limiter {
                ceiling_db,
                release_sec,
            } => {
                // alimiter takes linear limit, not dB.
                let limit_lin = 10f64.powf(*ceiling_db / 20.0);
                format!(
                    "alimiter=limit={}:level=disabled:release={}",
                    fmt_f(limit_lin),
                    fmt_f(release_sec * 1000.0)
                )
            }
            Self::Reverb { room_size, wet_db } => {
                // Map room_size 0..1 → 60..600ms decay via aecho.
                let decay = 0.06 + room_size.clamp(0.0, 1.0) * 0.54;
                let wet_lin = 10f64.powf(*wet_db / 20.0);
                format!(
                    "aecho=in_gain=0.8:out_gain={}:delays=60|120|240:decays={}|{}|{}",
                    fmt_f(wet_lin),
                    fmt_f(decay),
                    fmt_f(decay * 0.6),
                    fmt_f(decay * 0.3)
                )
            }
            Self::Gain { gain_db } => format!("volume={}dB", fmt_f(*gain_db)),
        }
    }

    /// Short human label for the UI ("Compressor", "De-esser", …).
    pub fn label(&self) -> &'static str {
        match self {
            Self::HighPass { .. } => "High-pass",
            Self::LowPass { .. } => "Low-pass",
            Self::EqBand { .. } => "EQ band",
            Self::NoiseReduction { .. } => "Noise reduction",
            Self::DeEsser { .. } => "De-esser",
            Self::Compressor { .. } => "Compressor",
            Self::Limiter { .. } => "Limiter",
            Self::Reverb { .. } => "Reverb",
            Self::Gain { .. } => "Gain",
        }
    }
}

/// Ordered insert chain — one per bus (voice / game / master).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct InsertChain {
    pub effects: Vec<InsertEffect>,
}

impl InsertChain {
    pub fn new(effects: Vec<InsertEffect>) -> Self {
        Self { effects }
    }

    /// True if the chain emits nothing — render skips it entirely.
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// Compose every effect into a single ffmpeg `-af` value.
    ///
    /// Empty chain → empty string (caller skips). Order matters: this is the
    /// signal flow, just like dragging plug-ins in Ableton or Logic.
    pub fn to_filter(&self) -> String {
        self.effects
            .iter()
            .map(InsertEffect::to_filter)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Vocal bus preset: HP @ 80 → NR 0.5 → de-esser @ 6k → 3:1 comp @ -18 →
    /// limiter @ -1. Sensible default for a single-mic talk stream.
    pub fn voice_bus_default() -> Self {
        Self::new(vec![
            InsertEffect::HighPass { freq_hz: 80.0 },
            InsertEffect::NoiseReduction { amount: 0.5 },
            InsertEffect::DeEsser {
                intensity: 0.5,
                freq_hz: 6000.0,
            },
            InsertEffect::Compressor {
                threshold_db: -18.0,
                ratio: 3.0,
                attack_sec: 0.005,
                release_sec: 0.15,
                makeup_db: 3.0,
            },
            InsertEffect::Limiter {
                ceiling_db: -1.0,
                release_sec: 0.05,
            },
        ])
    }

    /// Game bus preset: HP @ 40 → soft 2:1 comp @ -20 → limiter @ -1.
    /// Cleans rumble, glues stems, won't squash dialogue underneath.
    pub fn game_bus_default() -> Self {
        Self::new(vec![
            InsertEffect::HighPass { freq_hz: 40.0 },
            InsertEffect::Compressor {
                threshold_db: -20.0,
                ratio: 2.0,
                attack_sec: 0.01,
                release_sec: 0.25,
                makeup_db: 1.5,
            },
            InsertEffect::Limiter {
                ceiling_db: -1.0,
                release_sec: 0.08,
            },
        ])
    }
}

fn fmt_f(v: f64) -> String {
    // Match the existing automation/sidechain crates' formatting: trim
    // trailing zeros so test goldens stay readable.
    if v.fract() == 0.0 {
        format!("{:.1}", v)
    } else {
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highpass_filter_string() {
        let e = InsertEffect::HighPass { freq_hz: 80.0 };
        assert_eq!(e.to_filter(), "highpass=f=80.0");
    }

    #[test]
    fn lowpass_filter_string() {
        let e = InsertEffect::LowPass { freq_hz: 16000.0 };
        assert_eq!(e.to_filter(), "lowpass=f=16000.0");
    }

    #[test]
    fn eq_band_filter_string() {
        let e = InsertEffect::EqBand {
            freq_hz: 2500.0,
            gain_db: 4.5,
            width_q: 1.2,
        };
        assert_eq!(e.to_filter(), "equalizer=f=2500.0:t=q:w=1.2:g=4.5");
    }

    #[test]
    fn nr_maps_amount_to_db() {
        let mid = InsertEffect::NoiseReduction { amount: 0.5 };
        assert_eq!(mid.to_filter(), "afftdn=nr=18.0:nf=-25");
        let none = InsertEffect::NoiseReduction { amount: 0.0 };
        assert_eq!(none.to_filter(), "afftdn=nr=6.0:nf=-25");
        let full = InsertEffect::NoiseReduction { amount: 1.0 };
        assert_eq!(full.to_filter(), "afftdn=nr=30.0:nf=-25");
    }

    #[test]
    fn nr_clamps_out_of_range() {
        let e = InsertEffect::NoiseReduction { amount: 2.0 };
        assert!(e.to_filter().contains("nr=30.0"));
        let e2 = InsertEffect::NoiseReduction { amount: -1.0 };
        assert!(e2.to_filter().contains("nr=6.0"));
    }

    #[test]
    fn deesser_normalises_frequency() {
        let e = InsertEffect::DeEsser {
            intensity: 0.5,
            freq_hz: 6000.0,
        };
        // 6000/24000 = 0.25
        assert_eq!(e.to_filter(), "deesser=i=0.5:f=0.25:m=0.5:s=o");
    }

    #[test]
    fn compressor_attack_release_to_ms() {
        let e = InsertEffect::Compressor {
            threshold_db: -18.0,
            ratio: 3.0,
            attack_sec: 0.005,
            release_sec: 0.15,
            makeup_db: 3.0,
        };
        assert_eq!(
            e.to_filter(),
            "acompressor=threshold=-18.0dB:ratio=3.0:attack=5.0:release=150.0:makeup=3.0"
        );
    }

    #[test]
    fn limiter_db_to_linear() {
        let e = InsertEffect::Limiter {
            ceiling_db: 0.0,
            release_sec: 0.05,
        };
        // 0dB → linear 1.0
        assert_eq!(
            e.to_filter(),
            "alimiter=limit=1.0:level=disabled:release=50.0"
        );
    }

    #[test]
    fn empty_chain_emits_empty_string() {
        assert_eq!(InsertChain::default().to_filter(), "");
        assert!(InsertChain::default().is_empty());
    }

    #[test]
    fn chain_joins_with_commas_in_order() {
        let chain = InsertChain::new(vec![
            InsertEffect::HighPass { freq_hz: 80.0 },
            InsertEffect::Gain { gain_db: 2.0 },
            InsertEffect::Limiter {
                ceiling_db: -1.0,
                release_sec: 0.05,
            },
        ]);
        assert_eq!(
            chain.to_filter(),
            "highpass=f=80.0,volume=2.0dB,alimiter=limit=0.8913:level=disabled:release=50.0"
        );
    }

    #[test]
    fn voice_preset_has_expected_five_stages() {
        let v = InsertChain::voice_bus_default();
        assert_eq!(v.effects.len(), 5);
        assert_eq!(v.effects[0].label(), "High-pass");
        assert_eq!(v.effects[1].label(), "Noise reduction");
        assert_eq!(v.effects[2].label(), "De-esser");
        assert_eq!(v.effects[3].label(), "Compressor");
        assert_eq!(v.effects[4].label(), "Limiter");
    }

    #[test]
    fn game_preset_has_expected_three_stages() {
        let g = InsertChain::game_bus_default();
        assert_eq!(g.effects.len(), 3);
        assert_eq!(g.effects[0].label(), "High-pass");
        assert_eq!(g.effects[1].label(), "Compressor");
        assert_eq!(g.effects[2].label(), "Limiter");
    }

    #[test]
    fn round_trips_through_serde_json() {
        let chain = InsertChain::voice_bus_default();
        let json = serde_json::to_string(&chain).unwrap();
        let back: InsertChain = serde_json::from_str(&json).unwrap();
        assert_eq!(chain, back);
    }

    #[test]
    fn label_covers_every_variant() {
        // Catches the case where a new variant is added but the UI label
        // match arm is forgotten.
        for e in [
            InsertEffect::HighPass { freq_hz: 0.0 },
            InsertEffect::LowPass { freq_hz: 0.0 },
            InsertEffect::EqBand {
                freq_hz: 0.0,
                gain_db: 0.0,
                width_q: 1.0,
            },
            InsertEffect::NoiseReduction { amount: 0.0 },
            InsertEffect::DeEsser {
                intensity: 0.0,
                freq_hz: 0.0,
            },
            InsertEffect::Compressor {
                threshold_db: 0.0,
                ratio: 1.0,
                attack_sec: 0.0,
                release_sec: 0.0,
                makeup_db: 0.0,
            },
            InsertEffect::Limiter {
                ceiling_db: 0.0,
                release_sec: 0.0,
            },
            InsertEffect::Reverb {
                room_size: 0.0,
                wet_db: 0.0,
            },
            InsertEffect::Gain { gain_db: 0.0 },
        ] {
            assert!(!e.label().is_empty());
            assert!(!e.to_filter().is_empty());
        }
    }
}
