//! strivo-viewguard-trend — cross-stream Viewguard trend analyzer.
//!
//! The existing Viewguard plugin scores one stream at a time. Streamers
//! need the picture across N streams: is the fraud signal building?
//! Did blocking that one viewer help? Which channels just crossed the
//! "needs attention" threshold?
//!
//! This crate is **pure data** — takes a list of [`VerdictRow`]s and
//! returns the trend / watchlist / suggested-action shape the SPA
//! renders. No IO, no SQL, fully unit-testable.
//!
//! Key outputs:
//!
//!   * [`ChannelTrend`] — per-channel rolling mean, latest score,
//!     direction (Improving / Stable / Worsening), Δ delta, anomaly
//!     flag, sample count, suggested action.
//!   * [`Watchlist`] — channels grouped into Critical / Warning /
//!     Watch / Clear bands, sorted by latest score desc.
//!   * Per-band [`SuggestedAction`] copy lifted into a small enum so
//!     non-engineers can iterate without source edits.

use serde::{Deserialize, Serialize};

/// One Viewguard verdict row (matches the schema's `verdicts` table
/// shape we read out in the web crate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerdictRow {
    pub channel_id: String,
    pub channel_name: Option<String>,
    /// Final fraud score in [0, 1]. 1.0 = high confidence the audience
    /// is bot-padded.
    pub final_score: f32,
    /// ISO-8601 stream start time. Used for chronological sort.
    pub stream_started_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Improving,
    Stable,
    Worsening,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    Clear,
    Watch,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedAction {
    NoAction,
    KeepMonitoring,
    ManualReview,
    EscalateAndReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelTrend {
    pub channel_id: String,
    pub channel_name: String,
    pub samples: u32,
    pub latest_score: f32,
    pub rolling_mean: f32,
    pub delta: f32,
    pub direction: TrendDirection,
    pub anomaly: bool,
    pub band: Band,
    pub suggested_action: SuggestedAction,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Watchlist {
    pub critical: Vec<ChannelTrend>,
    pub warning: Vec<ChannelTrend>,
    pub watch: Vec<ChannelTrend>,
    pub clear: Vec<ChannelTrend>,
}

/// Score that flips a channel into each band. The default thresholds
/// reflect "score is 0..1 normalised confidence" — Critical at 70%,
/// Warning at 45%, Watch at 25%.
pub const CRITICAL_THRESHOLD: f32 = 0.70;
pub const WARNING_THRESHOLD: f32 = 0.45;
pub const WATCH_THRESHOLD: f32 = 0.25;

/// Delta magnitude that flags an anomaly when comparing the latest
/// verdict against the rolling mean.
pub const ANOMALY_DELTA: f32 = 0.20;

/// Build a [`Watchlist`] from a list of verdict rows.
pub fn build_watchlist(rows: &[VerdictRow]) -> Watchlist {
    let trends = build_trends(rows);
    let mut wl = Watchlist::default();
    for t in trends {
        match t.band {
            Band::Critical => wl.critical.push(t),
            Band::Warning => wl.warning.push(t),
            Band::Watch => wl.watch.push(t),
            Band::Clear => wl.clear.push(t),
        }
    }
    // Sort each band by latest_score desc so the noisiest channels
    // lead each section.
    for v in [&mut wl.critical, &mut wl.warning, &mut wl.watch, &mut wl.clear] {
        v.sort_by(|a, b| {
            b.latest_score
                .partial_cmp(&a.latest_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    wl
}

/// Build per-channel [`ChannelTrend`]s from verdict rows. Channels are
/// grouped by `channel_id`; samples within each group sorted oldest
/// to newest by `stream_started_at` string compare (ISO-8601 sorts
/// lexicographically when in Z form).
pub fn build_trends(rows: &[VerdictRow]) -> Vec<ChannelTrend> {
    use std::collections::HashMap;
    let mut by_channel: HashMap<String, Vec<VerdictRow>> = HashMap::new();
    for r in rows {
        by_channel
            .entry(r.channel_id.clone())
            .or_default()
            .push(r.clone());
    }
    let mut out: Vec<ChannelTrend> = Vec::new();
    for (channel_id, mut samples) in by_channel {
        samples.sort_by(|a, b| a.stream_started_at.cmp(&b.stream_started_at));
        let n = samples.len() as f32;
        if n == 0.0 {
            continue;
        }
        let latest = samples.last().unwrap();
        let latest_score = latest.final_score.clamp(0.0, 1.0);
        let channel_name = latest.channel_name.clone().unwrap_or_else(|| channel_id.clone());

        // Rolling mean of the last 5 samples (or fewer if fewer
        // exist), excluding the latest so direction compares the most
        // recent vs the recent history.
        let prior_window: Vec<f32> = samples
            .iter()
            .rev()
            .skip(1) // exclude latest
            .take(5)
            .map(|r| r.final_score.clamp(0.0, 1.0))
            .collect();
        let rolling_mean = if prior_window.is_empty() {
            latest_score
        } else {
            prior_window.iter().sum::<f32>() / prior_window.len() as f32
        };
        let delta = latest_score - rolling_mean;
        let direction = if delta < -0.05 {
            TrendDirection::Improving
        } else if delta > 0.05 {
            TrendDirection::Worsening
        } else {
            TrendDirection::Stable
        };
        let anomaly = delta.abs() >= ANOMALY_DELTA;
        let band = classify_band(latest_score);
        let suggested_action = recommend_action(band, anomaly);

        out.push(ChannelTrend {
            channel_id,
            channel_name,
            samples: samples.len() as u32,
            latest_score,
            rolling_mean,
            delta,
            direction,
            anomaly,
            band,
            suggested_action,
        });
    }
    // Sort overall by latest_score desc for stable enumeration in the
    // SPA (callers may further band/group).
    out.sort_by(|a, b| {
        b.latest_score
            .partial_cmp(&a.latest_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

pub fn classify_band(score: f32) -> Band {
    if score >= CRITICAL_THRESHOLD {
        Band::Critical
    } else if score >= WARNING_THRESHOLD {
        Band::Warning
    } else if score >= WATCH_THRESHOLD {
        Band::Watch
    } else {
        Band::Clear
    }
}

pub fn recommend_action(band: Band, anomaly: bool) -> SuggestedAction {
    match (band, anomaly) {
        (Band::Critical, _) => SuggestedAction::EscalateAndReport,
        (Band::Warning, true) => SuggestedAction::EscalateAndReport,
        (Band::Warning, false) => SuggestedAction::ManualReview,
        (Band::Watch, true) => SuggestedAction::ManualReview,
        (Band::Watch, false) => SuggestedAction::KeepMonitoring,
        (Band::Clear, true) => SuggestedAction::KeepMonitoring,
        (Band::Clear, false) => SuggestedAction::NoAction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(channel: &str, score: f32, ts: &str) -> VerdictRow {
        VerdictRow {
            channel_id: channel.into(),
            channel_name: Some(channel.to_string()),
            final_score: score,
            stream_started_at: ts.into(),
        }
    }

    #[test]
    fn empty_rows_yields_empty_watchlist() {
        let wl = build_watchlist(&[]);
        assert!(wl.critical.is_empty());
        assert!(wl.warning.is_empty());
        assert!(wl.watch.is_empty());
        assert!(wl.clear.is_empty());
    }

    #[test]
    fn classify_band_thresholds() {
        assert_eq!(classify_band(0.75), Band::Critical);
        assert_eq!(classify_band(0.50), Band::Warning);
        assert_eq!(classify_band(0.30), Band::Watch);
        assert_eq!(classify_band(0.10), Band::Clear);
        // Exact boundary lands in the higher band.
        assert_eq!(classify_band(CRITICAL_THRESHOLD), Band::Critical);
        assert_eq!(classify_band(WARNING_THRESHOLD), Band::Warning);
    }

    #[test]
    fn recommend_action_paths() {
        assert_eq!(recommend_action(Band::Critical, false), SuggestedAction::EscalateAndReport);
        assert_eq!(recommend_action(Band::Critical, true), SuggestedAction::EscalateAndReport);
        assert_eq!(recommend_action(Band::Warning, false), SuggestedAction::ManualReview);
        assert_eq!(recommend_action(Band::Warning, true), SuggestedAction::EscalateAndReport);
        assert_eq!(recommend_action(Band::Watch, false), SuggestedAction::KeepMonitoring);
        assert_eq!(recommend_action(Band::Watch, true), SuggestedAction::ManualReview);
        assert_eq!(recommend_action(Band::Clear, false), SuggestedAction::NoAction);
        assert_eq!(recommend_action(Band::Clear, true), SuggestedAction::KeepMonitoring);
    }

    #[test]
    fn single_sample_uses_score_as_rolling_mean() {
        let rows = vec![row("c1", 0.5, "2026-05-28T10:00:00Z")];
        let trends = build_trends(&rows);
        assert_eq!(trends.len(), 1);
        let t = &trends[0];
        assert_eq!(t.samples, 1);
        assert_eq!(t.latest_score, 0.5);
        assert_eq!(t.rolling_mean, 0.5);
        assert_eq!(t.delta, 0.0);
        assert_eq!(t.direction, TrendDirection::Stable);
    }

    #[test]
    fn worsening_direction_flags_rising_score() {
        let rows = vec![
            row("c1", 0.1, "2026-05-26T10:00:00Z"),
            row("c1", 0.15, "2026-05-27T10:00:00Z"),
            row("c1", 0.6, "2026-05-28T10:00:00Z"),
        ];
        let t = &build_trends(&rows)[0];
        assert_eq!(t.direction, TrendDirection::Worsening);
        assert!(t.delta > 0.05);
        assert!(t.anomaly, "0.6 vs ~0.125 mean → should be anomaly");
    }

    #[test]
    fn improving_direction_flags_falling_score() {
        let rows = vec![
            row("c1", 0.8, "2026-05-26T10:00:00Z"),
            row("c1", 0.75, "2026-05-27T10:00:00Z"),
            row("c1", 0.1, "2026-05-28T10:00:00Z"),
        ];
        let t = &build_trends(&rows)[0];
        assert_eq!(t.direction, TrendDirection::Improving);
        assert!(t.delta < -0.05);
    }

    #[test]
    fn stable_direction_when_delta_small() {
        let rows = vec![
            row("c1", 0.3, "2026-05-26T10:00:00Z"),
            row("c1", 0.32, "2026-05-27T10:00:00Z"),
            row("c1", 0.31, "2026-05-28T10:00:00Z"),
        ];
        let t = &build_trends(&rows)[0];
        assert_eq!(t.direction, TrendDirection::Stable);
    }

    #[test]
    fn rolling_mean_caps_to_last_five_priors() {
        // Eight samples; rolling mean should use the 5 most recent
        // *before* the latest, ignoring older outliers.
        let rows = vec![
            row("c1", 0.95, "2026-05-20T10:00:00Z"), // outlier, ignored
            row("c1", 0.95, "2026-05-21T10:00:00Z"), // outlier, ignored
            row("c1", 0.10, "2026-05-22T10:00:00Z"),
            row("c1", 0.12, "2026-05-23T10:00:00Z"),
            row("c1", 0.11, "2026-05-24T10:00:00Z"),
            row("c1", 0.15, "2026-05-25T10:00:00Z"),
            row("c1", 0.10, "2026-05-26T10:00:00Z"),
            row("c1", 0.20, "2026-05-27T10:00:00Z"),
        ];
        let t = &build_trends(&rows)[0];
        // Rolling mean is over the 5 priors before latest:
        // 0.10, 0.12, 0.11, 0.15, 0.10 → ~0.116
        assert!((t.rolling_mean - 0.116).abs() < 0.01, "got {}", t.rolling_mean);
    }

    #[test]
    fn watchlist_buckets_by_band() {
        let rows = vec![
            row("hot", 0.90, "2026-05-28T10:00:00Z"),
            row("warm", 0.50, "2026-05-28T10:00:00Z"),
            row("watch", 0.30, "2026-05-28T10:00:00Z"),
            row("calm", 0.05, "2026-05-28T10:00:00Z"),
        ];
        let wl = build_watchlist(&rows);
        assert_eq!(wl.critical.len(), 1);
        assert_eq!(wl.critical[0].channel_id, "hot");
        assert_eq!(wl.warning.len(), 1);
        assert_eq!(wl.warning[0].channel_id, "warm");
        assert_eq!(wl.watch.len(), 1);
        assert_eq!(wl.watch[0].channel_id, "watch");
        assert_eq!(wl.clear.len(), 1);
        assert_eq!(wl.clear[0].channel_id, "calm");
    }

    #[test]
    fn watchlist_sorts_within_band_by_score_desc() {
        let rows = vec![
            row("a", 0.71, "2026-05-28T10:00:00Z"),
            row("b", 0.85, "2026-05-28T10:00:00Z"),
            row("c", 0.95, "2026-05-28T10:00:00Z"),
        ];
        let wl = build_watchlist(&rows);
        assert_eq!(wl.critical.len(), 3);
        assert_eq!(wl.critical[0].channel_id, "c");
        assert_eq!(wl.critical[1].channel_id, "b");
        assert_eq!(wl.critical[2].channel_id, "a");
    }

    #[test]
    fn anomaly_flag_set_when_delta_exceeds_threshold() {
        let rows = vec![
            row("c1", 0.2, "2026-05-26T10:00:00Z"),
            row("c1", 0.22, "2026-05-27T10:00:00Z"),
            row("c1", 0.5, "2026-05-28T10:00:00Z"),
        ];
        let t = &build_trends(&rows)[0];
        assert!(t.anomaly);
    }

    #[test]
    fn negative_delta_anomaly_also_flags() {
        let rows = vec![
            row("c1", 0.9, "2026-05-26T10:00:00Z"),
            row("c1", 0.88, "2026-05-27T10:00:00Z"),
            row("c1", 0.4, "2026-05-28T10:00:00Z"),
        ];
        let t = &build_trends(&rows)[0];
        assert!(t.anomaly);
        assert_eq!(t.direction, TrendDirection::Improving);
    }

    #[test]
    fn build_trends_clamps_scores_to_unit() {
        let rows = vec![
            row("c1", -0.5, "2026-05-28T10:00:00Z"),
            row("c2", 2.0, "2026-05-28T10:00:00Z"),
        ];
        let trends = build_trends(&rows);
        for t in &trends {
            assert!((0.0..=1.0).contains(&t.latest_score));
        }
    }
}
