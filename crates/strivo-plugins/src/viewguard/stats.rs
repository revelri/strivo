//! Per-channel rolling stats + sample types.
//!
//! Each tracked-live channel keeps a 24h ring of 30-second viewer-count
//! samples in memory. The detectors read this directly; persistence is
//! handled separately in `store.rs`.
//!
//! 24h / 30s = 2880 samples per channel — ~23 KB per channel at u32, trivial.

use chrono::{DateTime, Utc};
use std::collections::VecDeque;

pub const BIN_SECS: i64 = 30;
pub const RING_CAPACITY: usize = 24 * 60 * 60 / 30; // 2880

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub ts: DateTime<Utc>,
    pub viewers: u32,
}

#[derive(Debug, Clone)]
pub struct ChannelStats {
    pub channel_id: String,
    pub platform: String,
    pub display_name: String,
    /// Stream session start. Reset on offline→live transitions so
    /// detectors gate themselves against minimum-sample-count.
    pub session_started_at: Option<DateTime<Utc>>,
    pub samples: VecDeque<Sample>,
}

impl ChannelStats {
    pub fn new(channel_id: String, platform: String, display_name: String) -> Self {
        Self {
            channel_id,
            platform,
            display_name,
            session_started_at: None,
            samples: VecDeque::with_capacity(RING_CAPACITY),
        }
    }

    /// Record a single sample. Quantizes ts to BIN_SECS so two pollers
    /// that land in the same bin idempotently overwrite. Returns true
    /// if a new sample was actually appended (false on dedupe).
    pub fn push(&mut self, ts: DateTime<Utc>, viewers: u32) -> bool {
        let bin_ts = quantize(ts);
        if let Some(last) = self.samples.back() {
            if last.ts == bin_ts {
                // dedupe — keep the latest viewer count for the bin
                let n = self.samples.len();
                self.samples[n - 1] = Sample { ts: bin_ts, viewers };
                return false;
            }
        }
        if self.samples.len() == RING_CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(Sample { ts: bin_ts, viewers });
        true
    }

    pub fn mark_session_start(&mut self, ts: DateTime<Utc>) {
        if self.session_started_at.is_none() {
            self.session_started_at = Some(ts);
        }
    }

    pub fn end_session(&mut self) {
        self.session_started_at = None;
    }

    /// Viewer values in chronological order — convenience for detectors.
    pub fn values(&self) -> Vec<u32> {
        self.samples.iter().map(|s| s.viewers).collect()
    }

    /// Most recent N samples, oldest-first.
    pub fn tail(&self, n: usize) -> Vec<Sample> {
        let start = self.samples.len().saturating_sub(n);
        self.samples.iter().skip(start).copied().collect()
    }
}

fn quantize(ts: DateTime<Utc>) -> DateTime<Utc> {
    let secs = ts.timestamp();
    let snapped = secs - (secs.rem_euclid(BIN_SECS));
    DateTime::<Utc>::from_timestamp(snapped, 0).unwrap_or(ts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(s, 0).unwrap()
    }

    #[test]
    fn dedup_within_bin() {
        let mut s = ChannelStats::new("c".into(), "twitch".into(), "C".into());
        assert!(s.push(t(100), 50));
        assert!(!s.push(t(115), 60)); // same 30s bin
        assert_eq!(s.samples.len(), 1);
        assert_eq!(s.samples.back().unwrap().viewers, 60); // latest wins
    }

    #[test]
    fn ring_capacity_holds() {
        let mut s = ChannelStats::new("c".into(), "twitch".into(), "C".into());
        for i in 0..(RING_CAPACITY as i64 + 100) {
            s.push(t(i * BIN_SECS), i as u32);
        }
        assert_eq!(s.samples.len(), RING_CAPACITY);
        // oldest evicted
        assert_eq!(s.samples.front().unwrap().viewers, 100);
    }

    #[test]
    fn quantize_snaps_down() {
        let ts = t(127);
        let q = quantize(ts);
        assert_eq!(q.timestamp(), 120);
    }

    #[test]
    fn session_lifecycle() {
        let mut s = ChannelStats::new("c".into(), "twitch".into(), "C".into());
        assert!(s.session_started_at.is_none());
        s.mark_session_start(t(1000));
        assert_eq!(s.session_started_at, Some(t(1000)));
        // idempotent
        s.mark_session_start(t(2000));
        assert_eq!(s.session_started_at, Some(t(1000)));
        s.end_session();
        assert!(s.session_started_at.is_none());
    }

    // Silence unused-import warning when only some tests are run.
    #[allow(dead_code)]
    fn _dur() -> Duration {
        Duration::seconds(1)
    }
}
