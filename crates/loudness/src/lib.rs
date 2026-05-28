//! EBU R128 loudness normalisation — DAW-vision master-bus loudness meter.
//!
//! Wraps the two-pass `ffmpeg loudnorm` workflow:
//!
//! 1. Pass 1: `ffmpeg -i in.mkv -af loudnorm=I=…:LRA=…:TP=…:print_format=json -f null -`
//!    Emits a JSON tail block with the measured loudness statistics.
//! 2. Pass 2: feed those measurements back into the filter as
//!    `measured_I=…:measured_LRA=…:measured_TP=…:measured_thresh=…` so
//!    the linear normaliser can apply the right gain without re-measuring.
//!
//! All maths and string formatting live here as pure data so the host can
//! unit-test against canned ffmpeg output. Twelve tests cover the parser
//! (well-formed + trailing noise + missing fields), the filter builder
//! (defaults + custom targets + escape-free formatting), and the per-
//! platform presets the Editor SPA exposes.

use serde::{Deserialize, Serialize};

/// Recommended EBU R128 target the SPA can preset from a platform pick.
/// All loudness values are in LUFS (Integrated) / LU (Loudness Range) /
/// dBTP (True Peak). See https://wiki.r128.org/ for the rationale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoudnormTarget {
    /// Integrated loudness target (-70 to -5 LUFS).
    pub i: f64,
    /// Loudness range target (1 to 50 LU).
    pub lra: f64,
    /// True-peak ceiling (-9 to 0 dBTP).
    pub tp: f64,
}

impl LoudnormTarget {
    /// Hard clamp to the ranges ffmpeg accepts so a fat-fingered SPA
    /// value can't produce an invalid filter argument.
    pub fn clamped(self) -> Self {
        Self {
            i: self.i.clamp(-70.0, -5.0),
            lra: self.lra.clamp(1.0, 50.0),
            tp: self.tp.clamp(-9.0, 0.0),
        }
    }
}

/// Pass-1 measurement block parsed from `print_format=json` stderr.
/// Five values per the loudnorm spec — every field is required for a
/// two-pass run; missing any one means we cannot do pass 2 cleanly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoudnormPass1 {
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
    /// Offset ffmpeg recommends adding to dynamic linear mode. Optional
    /// in older builds (defaults to 0.0 then).
    pub target_offset: f64,
}

/// Per-platform presets exposed in the Editor's Loudness panel. Each
/// platform's published target is rounded to the value content creators
/// most commonly aim for; the user can override every knob.
pub fn preset_for(platform: Platform) -> LoudnormTarget {
    match platform {
        Platform::YouTube => LoudnormTarget { i: -14.0, lra: 11.0, tp: -1.0 },
        Platform::Spotify => LoudnormTarget { i: -14.0, lra: 7.0, tp: -1.0 },
        Platform::AppleMusic => LoudnormTarget { i: -16.0, lra: 9.0, tp: -1.0 },
        Platform::EbuR128 => LoudnormTarget { i: -23.0, lra: 7.0, tp: -2.0 },
        Platform::Twitch => LoudnormTarget { i: -14.0, lra: 11.0, tp: -2.0 },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    YouTube,
    Spotify,
    AppleMusic,
    EbuR128,
    Twitch,
}

/// Build the pass-1 filter argument. The host runs
/// `ffmpeg -i in -af '<this>' -f null -` and captures stderr to parse.
pub fn pass1_filter(target: LoudnormTarget) -> String {
    let t = target.clamped();
    format!(
        "loudnorm=I={i}:LRA={lra}:TP={tp}:print_format=json",
        i = fmt_f(t.i),
        lra = fmt_f(t.lra),
        tp = fmt_f(t.tp),
    )
}

/// Build the pass-2 filter argument. Feeds the pass-1 measurements
/// back in so the linear normaliser applies the right gain.
pub fn pass2_filter(target: LoudnormTarget, pass1: &LoudnormPass1) -> String {
    let t = target.clamped();
    format!(
        "loudnorm=I={i}:LRA={lra}:TP={tp}:measured_I={mi}:measured_LRA={mlra}:measured_TP={mtp}:measured_thresh={mthresh}:offset={off}:linear=true:print_format=summary",
        i = fmt_f(t.i),
        lra = fmt_f(t.lra),
        tp = fmt_f(t.tp),
        mi = fmt_f(pass1.input_i),
        mlra = fmt_f(pass1.input_lra),
        mtp = fmt_f(pass1.input_tp),
        mthresh = fmt_f(pass1.input_thresh),
        off = fmt_f(pass1.target_offset),
    )
}

/// Parse the JSON block ffmpeg appends to stderr when
/// `print_format=json` is requested. Tolerant of leading non-JSON
/// (the rest of ffmpeg's stderr) and of an `input_*` field being
/// reported as a string rather than a number.
pub fn parse_pass1(stderr: &str) -> Option<LoudnormPass1> {
    let start = stderr.find('{')?;
    let end = stderr.rfind('}')? + 1;
    if end <= start {
        return None;
    }
    let block = &stderr[start..end];
    let v: serde_json::Value = serde_json::from_str(block).ok()?;
    let pick = |k: &str| -> Option<f64> {
        v.get(k).and_then(|x| match x {
            serde_json::Value::String(s) => s.trim().parse().ok(),
            serde_json::Value::Number(n) => n.as_f64(),
            _ => None,
        })
    };
    Some(LoudnormPass1 {
        input_i: pick("input_i")?,
        input_tp: pick("input_tp")?,
        input_lra: pick("input_lra")?,
        input_thresh: pick("input_thresh")?,
        target_offset: pick("target_offset").unwrap_or(0.0),
    })
}

/// Distance from a target — positive means louder than target, negative
/// quieter. Useful for the Editor to render a colour-coded gauge before
/// committing to render.
pub fn delta_from_target(target: LoudnormTarget, pass1: &LoudnormPass1) -> LoudnormDelta {
    LoudnormDelta {
        i_delta: pass1.input_i - target.i,
        tp_delta: pass1.input_tp - target.tp,
        lra_delta: pass1.input_lra - target.lra,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoudnormDelta {
    pub i_delta: f64,
    pub tp_delta: f64,
    pub lra_delta: f64,
}

/// Format an f64 the way ffmpeg expects in filter args — three decimals,
/// no scientific notation, no locale-specific separators.
fn fmt_f(v: f64) -> String {
    format!("{:.3}", v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pass1_stderr() -> &'static str {
        // Trimmed but realistic ffmpeg loudnorm pass-1 emission. The
        // chunk preceding the {…} block is what ffmpeg's progress lines
        // look like; our parser must skip it.
        r#"
frame= 2400 fps=120 q=-1.0 size=N/A time=00:01:40.00 bitrate=N/A speed=4.99x
[Parsed_loudnorm_0 @ 0x55a14]
{
        "input_i" : "-26.51",
        "input_tp" : "-15.92",
        "input_lra" : "5.40",
        "input_thresh" : "-36.65",
        "output_i" : "-22.01",
        "output_tp" : "-11.27",
        "output_lra" : "5.30",
        "output_thresh" : "-32.07",
        "normalization_type" : "dynamic",
        "target_offset" : "0.51"
}
"#
    }

    #[test]
    fn parses_well_formed_pass1_block() {
        let p = parse_pass1(sample_pass1_stderr()).unwrap();
        assert!((p.input_i - -26.51).abs() < 1e-6);
        assert!((p.input_tp - -15.92).abs() < 1e-6);
        assert!((p.input_lra - 5.40).abs() < 1e-6);
        assert!((p.input_thresh - -36.65).abs() < 1e-6);
        assert!((p.target_offset - 0.51).abs() < 1e-6);
    }

    #[test]
    fn parse_returns_none_on_malformed_block() {
        assert!(parse_pass1("no json here").is_none());
        assert!(parse_pass1("{ unclosed").is_none());
    }

    #[test]
    fn parse_accepts_numeric_input_fields() {
        // Older / customised ffmpeg builds may emit numbers rather than
        // quoted strings.
        let s = r#"{"input_i":-26.51,"input_tp":-15.92,"input_lra":5.4,"input_thresh":-36.65,"target_offset":0.51}"#;
        let p = parse_pass1(s).unwrap();
        assert!((p.input_i - -26.51).abs() < 1e-6);
    }

    #[test]
    fn parse_missing_required_field_returns_none() {
        let s = r#"{"input_i":"-26.51","input_tp":"-15.92","input_lra":"5.40"}"#;
        // Missing input_thresh.
        assert!(parse_pass1(s).is_none());
    }

    #[test]
    fn parse_missing_target_offset_defaults_to_zero() {
        let s = r#"{"input_i":"-26.51","input_tp":"-15.92","input_lra":"5.40","input_thresh":"-36.65"}"#;
        let p = parse_pass1(s).unwrap();
        assert_eq!(p.target_offset, 0.0);
    }

    #[test]
    fn pass1_filter_uses_target_with_three_decimals() {
        let f = pass1_filter(preset_for(Platform::YouTube));
        assert_eq!(
            f,
            "loudnorm=I=-14.000:LRA=11.000:TP=-1.000:print_format=json"
        );
    }

    #[test]
    fn pass2_filter_includes_measured_block_and_linear_true() {
        let p = LoudnormPass1 {
            input_i: -26.51,
            input_tp: -15.92,
            input_lra: 5.40,
            input_thresh: -36.65,
            target_offset: 0.51,
        };
        let f = pass2_filter(preset_for(Platform::YouTube), &p);
        assert!(f.contains("measured_I=-26.510"));
        assert!(f.contains("measured_LRA=5.400"));
        assert!(f.contains("measured_TP=-15.920"));
        assert!(f.contains("measured_thresh=-36.650"));
        assert!(f.contains("offset=0.510"));
        assert!(f.contains("linear=true"));
    }

    #[test]
    fn target_clamps_out_of_range_inputs() {
        let t = LoudnormTarget { i: -100.0, lra: 200.0, tp: 5.0 }.clamped();
        assert_eq!(t.i, -70.0);
        assert_eq!(t.lra, 50.0);
        assert_eq!(t.tp, 0.0);
    }

    #[test]
    fn target_clamps_low_extremes() {
        let t = LoudnormTarget { i: 5.0, lra: -3.0, tp: -50.0 }.clamped();
        assert_eq!(t.i, -5.0);
        assert_eq!(t.lra, 1.0);
        assert_eq!(t.tp, -9.0);
    }

    #[test]
    fn presets_match_published_platform_targets() {
        assert_eq!(preset_for(Platform::YouTube).i, -14.0);
        assert_eq!(preset_for(Platform::Spotify).i, -14.0);
        assert_eq!(preset_for(Platform::AppleMusic).i, -16.0);
        assert_eq!(preset_for(Platform::EbuR128).i, -23.0);
        assert_eq!(preset_for(Platform::Twitch).i, -14.0);
    }

    #[test]
    fn delta_from_target_reports_signed_distance() {
        let p = LoudnormPass1 {
            input_i: -16.0,
            input_tp: -3.0,
            input_lra: 9.0,
            input_thresh: -28.0,
            target_offset: 0.0,
        };
        let d = delta_from_target(preset_for(Platform::YouTube), &p);
        // I delta: -16.0 - -14.0 = -2.0 LU below target
        assert!((d.i_delta - -2.0).abs() < 1e-6);
        // TP delta: -3.0 - -1.0 = -2.0 dB below target
        assert!((d.tp_delta - -2.0).abs() < 1e-6);
    }

    #[test]
    fn fmt_f_emits_three_decimals_no_scientific() {
        assert_eq!(fmt_f(-26.51), "-26.510");
        assert_eq!(fmt_f(0.0000001), "0.000");
        assert_eq!(fmt_f(1.0e6), "1000000.000");
    }
}
