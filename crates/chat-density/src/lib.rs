//! strivo-chat-density — chat-density-derived audience retention.
//!
//! Promotes the second marketplace catalog entry (twitch-chat-density)
//! from "coming soon" to "ships today". Provides the canonical
//! audience-retention signal the iter 13 Heatmap plugin reserved a
//! slot for via the `x.chat_density` capability extension.
//!
//! Two ingestion paths:
//!
//!   * [`parse_irc_log`] reads a Twitch IRC dump line by line. Each
//!     `PRIVMSG #channel :message` line becomes a [`ChatEvent`] when a
//!     `@badge-info=...;tmi-sent-ts=<ms>` tag block is present (the
//!     standard Twitch chatty-formatted dump shape).
//!   * [`parse_csv_log`] reads a simple CSV (`time_sec,user,message`
//!     header optional) for non-Twitch sources or hand-edited logs.
//!
//! Both produce a `Vec<ChatEvent>` the analyzer then folds:
//!
//!   * [`compute_density`] buckets events into `bucket_secs` windows,
//!     counts messages, deduplicates unique chatters, computes a
//!     weighted score that boosts buckets with high unique-chatter
//!     turnover (broader engagement), and normalises both density and
//!     score to [0, 1] across the recording.
//!
//! All pure — no IO, no clock; tests feed canned strings.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEvent {
    pub time_sec: f32,
    pub user: String,
    /// Free-form message text. Kept so downstream filters can keyword-
    /// match (raids, sponsor mentions, etc.); not used by the analyzer.
    #[serde(default)]
    pub message: String,
    /// Relative weight. Defaults to 1.0; channel bots and unverified
    /// accounts can be down-weighted by the caller before computing.
    #[serde(default = "default_weight")]
    pub weight: f32,
}

fn default_weight() -> f32 { 1.0 }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DensityPoint {
    pub bucket_start: f32,
    pub message_count: u32,
    pub unique_chatters: u32,
    /// Normalised [0,1] message-density (per bucket vs the loudest one).
    pub density: f32,
    /// Normalised engagement: leans on unique chatters so a bot spam
    /// doesn't dominate. Fused as 0.6·density + 0.4·unique-density.
    pub engagement: f32,
}

/// Parse a Twitch IRC dump. Recognises both `tmi-sent-ts` epoch-ms
/// tags and an explicit `time_sec=` annotation for non-Twitch
/// sources. `stream_start_ts_ms` is the epoch-ms timestamp the stream
/// started — events are emitted with `time_sec = (event_ts - start)/1000`.
pub fn parse_irc_log(log: &str, stream_start_ts_ms: u64) -> Vec<ChatEvent> {
    let mut out: Vec<ChatEvent> = Vec::new();
    for line in log.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Anatomy: @<tags> :<nick>!<...>@<...>.tmi.twitch.tv PRIVMSG #<channel> :<message>
        if !line.contains("PRIVMSG") {
            continue;
        }
        let tags = if line.starts_with('@') {
            let end = line.find(' ').unwrap_or(line.len());
            &line[1..end]
        } else {
            ""
        };
        let ts_ms = tag_value(tags, "tmi-sent-ts").and_then(|v| v.parse::<u64>().ok());
        let nick = parse_nick(line).unwrap_or_default();
        let message = parse_message(line).unwrap_or_default();
        if nick.is_empty() {
            continue;
        }
        let time_sec = match ts_ms {
            Some(ms) if ms >= stream_start_ts_ms => (ms - stream_start_ts_ms) as f32 / 1000.0,
            // No usable timestamp — skip so the analyzer doesn't pile
            // every untimestamped event into bucket 0.
            _ => continue,
        };
        out.push(ChatEvent {
            time_sec,
            user: nick,
            message,
            weight: 1.0,
        });
    }
    out
}

fn tag_value<'a>(tags: &'a str, key: &str) -> Option<&'a str> {
    for pair in tags.split(';') {
        let mut it = pair.splitn(2, '=');
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            if k == key {
                return Some(v);
            }
        }
    }
    None
}

fn parse_nick(line: &str) -> Option<String> {
    // Two shapes:
    //  ":nick!user@host.tmi.twitch.tv PRIVMSG ..."
    //  "@tags ... :nick!user@host.tmi.twitch.tv PRIVMSG ..."
    let priv_idx = line.find("PRIVMSG")?;
    let before = &line[..priv_idx];
    let last_colon = before.rfind(':')?;
    let prefix = &before[last_colon + 1..];
    let bang = prefix.find('!')?;
    Some(prefix[..bang].to_string())
}

fn parse_message(line: &str) -> Option<String> {
    // Locate the SECOND `:` (after PRIVMSG #channel) — that's the
    // start of the message body.
    let priv_idx = line.find("PRIVMSG")?;
    let after = &line[priv_idx..];
    let colon = after.find(" :")?;
    Some(after[colon + 2..].to_string())
}

/// Parse a `time_sec,user,message` CSV. Header row optional (detected
/// by the first column being non-numeric). Empty / malformed lines
/// are skipped silently.
pub fn parse_csv_log(csv: &str) -> Vec<ChatEvent> {
    let mut out: Vec<ChatEvent> = Vec::new();
    for (i, raw) in csv.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, ',').collect();
        if parts.len() < 2 {
            continue;
        }
        // Header detection: first row's first column not a number.
        let time_sec: f32 = match parts[0].trim().parse() {
            Ok(n) => n,
            Err(_) if i == 0 => continue, // header row
            Err(_) => continue,
        };
        let user = parts[1].trim();
        if user.is_empty() {
            continue;
        }
        let message = parts.get(2).map(|s| s.trim().to_string()).unwrap_or_default();
        out.push(ChatEvent {
            time_sec,
            user: user.to_string(),
            message,
            weight: 1.0,
        });
    }
    out
}

/// Bucket events and compute density / engagement curves.
pub fn compute_density(
    events: &[ChatEvent],
    duration_sec: f32,
    bucket_secs: f32,
) -> Vec<DensityPoint> {
    if duration_sec <= 0.0 || bucket_secs <= 0.0 {
        return Vec::new();
    }
    let n = (duration_sec / bucket_secs).ceil() as usize;
    if n == 0 {
        return Vec::new();
    }
    let mut counts = vec![0.0_f32; n];
    let mut chatters: Vec<BTreeSet<String>> = vec![BTreeSet::new(); n];
    for ev in events {
        if ev.time_sec < 0.0 {
            continue;
        }
        let b = (ev.time_sec / bucket_secs).floor() as usize;
        if b < n {
            counts[b] += ev.weight.max(0.0);
            chatters[b].insert(ev.user.clone());
        }
    }
    let unique_counts: Vec<f32> = chatters.iter().map(|s| s.len() as f32).collect();
    let max_count = counts.iter().copied().fold(0.0_f32, f32::max).max(1.0);
    let max_unique = unique_counts.iter().copied().fold(0.0_f32, f32::max).max(1.0);
    (0..n)
        .map(|i| {
            let density = counts[i] / max_count;
            let unique_norm = unique_counts[i] / max_unique;
            let engagement = 0.6 * density + 0.4 * unique_norm;
            DensityPoint {
                bucket_start: i as f32 * bucket_secs,
                message_count: counts[i] as u32,
                unique_chatters: chatters[i].len() as u32,
                density,
                engagement: engagement.clamp(0.0, 1.0),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_skips_header_and_handles_missing_message() {
        let csv = "time_sec,user,message\n12.5,alice,hello\n42.0,bob,";
        let evs = parse_csv_log(csv);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].user, "alice");
        assert!((evs[0].time_sec - 12.5).abs() < 1e-5);
        assert_eq!(evs[1].message, "");
    }

    #[test]
    fn parse_csv_handles_numeric_first_row_as_data() {
        let csv = "5,alice,hi";
        let evs = parse_csv_log(csv);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].user, "alice");
    }

    #[test]
    fn parse_csv_skips_malformed_lines() {
        let csv = "5,alice,hi\nbroken\n10,bob,";
        let evs = parse_csv_log(csv);
        assert_eq!(evs.len(), 2);
    }

    #[test]
    fn parse_irc_skips_lines_without_privmsg() {
        let log = "PING :tmi.twitch.tv\n@badge-info=;tmi-sent-ts=1000 :nick!nick@nick.tmi.twitch.tv JOIN #chan\n";
        let evs = parse_irc_log(log, 0);
        assert!(evs.is_empty());
    }

    #[test]
    fn parse_irc_extracts_nick_and_message_with_timestamp() {
        let log = "@badge-info=;tmi-sent-ts=1500 :alice!alice@alice.tmi.twitch.tv PRIVMSG #chan :hello world\n";
        let evs = parse_irc_log(log, 1000);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].user, "alice");
        assert_eq!(evs[0].message, "hello world");
        assert!((evs[0].time_sec - 0.5).abs() < 1e-3);
    }

    #[test]
    fn parse_irc_skips_events_before_stream_start() {
        // Event ts (500ms) is before the stream-start (1000ms) — should
        // be discarded so it doesn't pile into bucket 0.
        let log = "@badge-info=;tmi-sent-ts=500 :alice!alice@alice.tmi.twitch.tv PRIVMSG #chan :hi\n";
        let evs = parse_irc_log(log, 1000);
        assert!(evs.is_empty());
    }

    #[test]
    fn parse_irc_skips_events_with_no_timestamp() {
        let log = ":alice!alice@alice.tmi.twitch.tv PRIVMSG #chan :hi\n";
        let evs = parse_irc_log(log, 0);
        assert!(evs.is_empty(), "got {evs:?}");
    }

    #[test]
    fn density_returns_empty_for_zero_duration() {
        let evs = vec![ChatEvent { time_sec: 0.0, user: "a".into(), message: "".into(), weight: 1.0 }];
        assert!(compute_density(&evs, 0.0, 30.0).is_empty());
    }

    #[test]
    fn density_buckets_events_correctly() {
        let evs = vec![
            ChatEvent { time_sec: 5.0, user: "a".into(), message: "".into(), weight: 1.0 },
            ChatEvent { time_sec: 7.0, user: "b".into(), message: "".into(), weight: 1.0 },
            ChatEvent { time_sec: 35.0, user: "a".into(), message: "".into(), weight: 1.0 },
        ];
        let pts = compute_density(&evs, 60.0, 30.0);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].message_count, 2);
        assert_eq!(pts[0].unique_chatters, 2);
        assert_eq!(pts[1].message_count, 1);
    }

    #[test]
    fn density_engagement_lifts_high_unique_chatters() {
        // Bucket 0 has 10 messages from 10 unique chatters; bucket 1
        // has 10 messages from 1 chatter (bot). Engagement should
        // rank bucket 0 above bucket 1 even though message_count ties.
        let mut evs: Vec<ChatEvent> = Vec::new();
        for i in 0..10 {
            evs.push(ChatEvent {
                time_sec: i as f32,
                user: format!("user_{i}"),
                message: "".into(),
                weight: 1.0,
            });
        }
        for _ in 0..10 {
            evs.push(ChatEvent {
                time_sec: 35.0,
                user: "spambot".into(),
                message: "".into(),
                weight: 1.0,
            });
        }
        let pts = compute_density(&evs, 60.0, 30.0);
        assert_eq!(pts.len(), 2);
        assert!(pts[0].engagement > pts[1].engagement);
    }

    #[test]
    fn density_clamps_to_unit_range() {
        let evs = (0..50)
            .map(|i| ChatEvent {
                time_sec: (i as f32) * 1.0,
                user: format!("u{}", i % 5),
                message: "".into(),
                weight: 1.0,
            })
            .collect::<Vec<_>>();
        let pts = compute_density(&evs, 60.0, 30.0);
        for p in &pts {
            assert!((0.0..=1.0).contains(&p.density));
            assert!((0.0..=1.0).contains(&p.engagement));
        }
    }

    #[test]
    fn density_negative_event_times_ignored() {
        let evs = vec![
            ChatEvent { time_sec: -5.0, user: "a".into(), message: "".into(), weight: 1.0 },
            ChatEvent { time_sec: 10.0, user: "b".into(), message: "".into(), weight: 1.0 },
        ];
        let pts = compute_density(&evs, 60.0, 30.0);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].message_count, 1);
    }

    #[test]
    fn density_respects_event_weight() {
        // Two messages in bucket 0 with weight 0.5 each → message_count
        // floored to 1 (sum = 1.0 → as u32 = 1).
        let evs = vec![
            ChatEvent { time_sec: 1.0, user: "a".into(), message: "".into(), weight: 0.5 },
            ChatEvent { time_sec: 2.0, user: "b".into(), message: "".into(), weight: 0.5 },
        ];
        let pts = compute_density(&evs, 30.0, 30.0);
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].message_count, 1);
        assert_eq!(pts[0].unique_chatters, 2);
    }

    #[test]
    fn density_unique_chatters_dedupes_users() {
        let evs = vec![
            ChatEvent { time_sec: 1.0, user: "alice".into(), message: "".into(), weight: 1.0 },
            ChatEvent { time_sec: 2.0, user: "alice".into(), message: "".into(), weight: 1.0 },
            ChatEvent { time_sec: 3.0, user: "alice".into(), message: "".into(), weight: 1.0 },
        ];
        let pts = compute_density(&evs, 30.0, 30.0);
        assert_eq!(pts[0].message_count, 3);
        assert_eq!(pts[0].unique_chatters, 1);
    }
}
