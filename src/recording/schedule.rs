use std::collections::HashMap;
use std::str::FromStr;

use chrono::Utc;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app::AppEvent;
use crate::config::AppConfig;
use crate::platform::PlatformKind;
use crate::recording::RecordingCommand;

/// Persistent schedule state — survives restarts to prevent duplicate recordings.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ScheduleState {
    /// Map from schedule key (channel + cron) to last triggered time.
    last_triggered: HashMap<String, chrono::DateTime<Utc>>,
}

impl ScheduleState {
    fn state_path() -> std::path::PathBuf {
        AppConfig::state_dir().join("schedule-state.json")
    }

    fn load() -> Self {
        let path = Self::state_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn save(&self) {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(contents) = serde_json::to_string_pretty(self) {
            if let Err(e) = std::fs::write(&path, contents) {
                tracing::warn!("Failed to persist schedule state: {e}");
            }
        }
    }

    /// Returns a unique key for a schedule entry.
    fn key(channel: &str, cron: &str) -> String {
        format!("{channel}|{cron}")
    }

    /// Check if this schedule was already triggered within the last `window_secs`.
    fn was_triggered_recently(&self, key: &str, window_secs: i64) -> bool {
        if let Some(last) = self.last_triggered.get(key) {
            let elapsed = (Utc::now() - *last).num_seconds();
            elapsed < window_secs
        } else {
            false
        }
    }

    fn mark_triggered(&mut self, key: &str) {
        self.last_triggered.insert(key.to_string(), Utc::now());
        self.save();
    }
}

struct ActiveSchedule {
    entry: crate::config::ScheduleEntry,
    schedule: Schedule,
    platform: PlatformKind,
    channel_name: String,
    state_key: String,
    /// Pre-generated job_id for the current recording (so we can send Stop).
    job_id: Option<Uuid>,
    /// When the current scheduled recording should stop.
    stop_at: Option<chrono::DateTime<Utc>>,
    /// Whether a recording has been started for the current window.
    recording_active: bool,
}

/// Parse a duration string like "4h", "30m", "2h30m", "90m" into seconds.
pub fn parse_duration_secs(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    let mut total: u64 = 0;
    let mut current = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            current.push(c);
        } else {
            let n: u64 = current.parse().ok()?;
            current.clear();
            match c {
                'h' => total += n * 3600,
                'm' => total += n * 60,
                's' => total += n,
                _ => return None,
            }
        }
    }

    // Handle bare number (treat as minutes)
    if !current.is_empty() {
        let n: u64 = current.parse().ok()?;
        total += n * 60;
    }

    if total > 0 {
        Some(total)
    } else {
        None
    }
}

/// Parse "twitch:channelname" or "youtube:channelname" into (PlatformKind, channel_name).
pub fn parse_channel_spec(spec: &str) -> Option<(PlatformKind, String)> {
    if let Some((platform, name)) = spec.split_once(':') {
        let kind = match platform.to_lowercase().as_str() {
            "twitch" | "tw" => PlatformKind::Twitch,
            "youtube" | "yt" => PlatformKind::YouTube,
            "patreon" | "pa" => PlatformKind::Patreon,
            _ => return None,
        };
        Some((kind, name.to_string()))
    } else {
        // Default to Twitch if no platform prefix
        Some((PlatformKind::Twitch, spec.to_string()))
    }
}

/// Runs the schedule manager, evaluating cron expressions and starting/stopping recordings.
pub async fn run_schedule_manager(
    config: AppConfig,
    recording_tx: mpsc::UnboundedSender<RecordingCommand>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    cancel: CancellationToken,
) {
    if config.schedule.is_empty() {
        return;
    }

    let mut state = ScheduleState::load();
    let mut schedules: Vec<ActiveSchedule> = Vec::new();

    for entry in &config.schedule {
        let cron_expr = if entry.cron.split_whitespace().count() == 5 {
            // Standard 5-field cron — prepend "0" seconds field
            format!("0 {}", entry.cron)
        } else {
            entry.cron.clone()
        };

        let schedule = match Schedule::from_str(&cron_expr) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    "Invalid cron expression '{}' for {}: {e}",
                    entry.cron,
                    entry.channel
                );
                let _ = event_tx.send(AppEvent::error(format!(
                    "Invalid schedule cron '{}': {e}",
                    entry.cron
                )));
                continue;
            }
        };

        let (platform, channel_name) = match parse_channel_spec(&entry.channel) {
            Some(p) => p,
            None => {
                tracing::error!("Invalid channel spec '{}' in schedule", entry.channel);
                continue;
            }
        };

        let state_key = ScheduleState::key(&entry.channel, &entry.cron);

        tracing::info!(
            "Schedule registered: {} ({}), cron: {}, duration: {}",
            channel_name,
            platform,
            entry.cron,
            entry.duration,
        );

        schedules.push(ActiveSchedule {
            entry: entry.clone(),
            schedule,
            platform,
            channel_name,
            state_key,
            job_id: None,
            stop_at: None,
            recording_active: false,
        });
    }

    if schedules.is_empty() {
        return;
    }

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = Utc::now();

                for sched in &mut schedules {
                    // Check if we need to stop an active scheduled recording
                    if sched.recording_active {
                        if let Some(stop_at) = sched.stop_at {
                            if now >= stop_at {
                                tracing::info!(
                                    "Schedule: stopping recording for {} (duration reached)",
                                    sched.channel_name,
                                );
                                // Send Stop with the pre-generated job_id
                                if let Some(job_id) = sched.job_id.take() {
                                    let _ = recording_tx.send(RecordingCommand::Stop { job_id });
                                }
                                sched.recording_active = false;
                                sched.stop_at = None;
                            }
                        }
                        continue;
                    }

                    // Check if current time matches a cron window
                    // Look at the upcoming occurrence — if it's within the next 60 seconds, start
                    if let Some(next) = sched.schedule.upcoming(Utc).next() {
                        let diff = (next - now).num_seconds();
                        if diff <= 60 && diff >= -60 {
                            // Prevent duplicate: check if we triggered this window recently
                            // Use a window slightly larger than the poll interval to avoid edge cases
                            if state.was_triggered_recently(&sched.state_key, 120) {
                                continue;
                            }

                            let duration_secs = parse_duration_secs(&sched.entry.duration)
                                .unwrap_or_else(|| {
                                    tracing::warn!(
                                        "Failed to parse duration '{}' for schedule {}, defaulting to 4h",
                                        sched.entry.duration, sched.channel_name,
                                    );
                                    4 * 3600
                                });

                            // Pre-generate job_id so we can send Stop later
                            let job_id = Uuid::new_v4();

                            // Channel may be offline — that's expected for scheduled recordings.
                            // The recording manager will handle the offline case (ResolvingUrl → Failed).
                            tracing::info!(
                                "Schedule: starting recording for {} ({}), duration: {}s, job_id: {}",
                                sched.channel_name,
                                sched.platform,
                                duration_secs,
                                job_id,
                            );

                            let _ = recording_tx.send(RecordingCommand::Start {
                                channel_id: sched.channel_name.clone(),
                                channel_name: sched.channel_name.clone(),
                                platform: sched.platform,
                                transcode: false,
                                cookies_path: None,
                                stream_title: Some("Scheduled recording".to_string()),
                                from_start: false,
                                job_id: Some(job_id),
                            });

                            let _ = event_tx.send(AppEvent::schedule_fired(
                                sched.channel_name.clone(),
                                sched.platform,
                                job_id,
                                duration_secs,
                            ));
                            let _ = event_tx.send(AppEvent::notification(
                                "Scheduled Recording".to_string(),
                                format!("Starting scheduled recording: {}", sched.channel_name),
                            ));

                            sched.recording_active = true;
                            sched.job_id = Some(job_id);
                            sched.stop_at = Some(
                                now + chrono::Duration::seconds(duration_secs as i64),
                            );

                            // Persist trigger time to survive restarts
                            state.mark_triggered(&sched.state_key);
                        }
                    }
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("Schedule manager shutting down");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_secs_hours() {
        assert_eq!(parse_duration_secs("4h"), Some(14400));
        assert_eq!(parse_duration_secs("1h"), Some(3600));
    }

    #[test]
    fn test_parse_duration_secs_minutes() {
        assert_eq!(parse_duration_secs("30m"), Some(1800));
        assert_eq!(parse_duration_secs("90m"), Some(5400));
    }

    #[test]
    fn test_parse_duration_secs_combined() {
        assert_eq!(parse_duration_secs("2h30m"), Some(9000));
        assert_eq!(parse_duration_secs("1h15m30s"), Some(4530));
    }

    #[test]
    fn test_parse_duration_secs_bare_number() {
        // Bare number treated as minutes
        assert_eq!(parse_duration_secs("60"), Some(3600));
        assert_eq!(parse_duration_secs("120"), Some(7200));
    }

    #[test]
    fn test_parse_duration_secs_seconds() {
        assert_eq!(parse_duration_secs("90s"), Some(90));
    }

    #[test]
    fn test_parse_duration_secs_whitespace() {
        assert_eq!(parse_duration_secs("  4h  "), Some(14400));
    }

    #[test]
    fn test_parse_duration_secs_invalid() {
        assert_eq!(parse_duration_secs(""), None);
        assert_eq!(parse_duration_secs("abc"), None);
        assert_eq!(parse_duration_secs("0"), None);
        assert_eq!(parse_duration_secs("0h"), None);
    }

    #[test]
    fn test_parse_channel_spec_twitch() {
        let (kind, name) = parse_channel_spec("twitch:shroud").unwrap();
        assert!(matches!(kind, PlatformKind::Twitch));
        assert_eq!(name, "shroud");
    }

    #[test]
    fn test_parse_channel_spec_youtube() {
        let (kind, name) = parse_channel_spec("youtube:PewDiePie").unwrap();
        assert!(matches!(kind, PlatformKind::YouTube));
        assert_eq!(name, "PewDiePie");
    }

    #[test]
    fn test_parse_channel_spec_short_prefix() {
        let (kind, name) = parse_channel_spec("tw:ninja").unwrap();
        assert!(matches!(kind, PlatformKind::Twitch));
        assert_eq!(name, "ninja");

        let (kind, name) = parse_channel_spec("yt:mkbhd").unwrap();
        assert!(matches!(kind, PlatformKind::YouTube));
        assert_eq!(name, "mkbhd");
    }

    #[test]
    fn test_parse_channel_spec_no_prefix_defaults_twitch() {
        let (kind, name) = parse_channel_spec("xqc").unwrap();
        assert!(matches!(kind, PlatformKind::Twitch));
        assert_eq!(name, "xqc");
    }

    #[test]
    fn test_parse_channel_spec_invalid_platform() {
        assert!(parse_channel_spec("invalid:foo").is_none());
    }

    #[test]
    fn test_parse_channel_spec_patreon() {
        let (kind, name) = parse_channel_spec("patreon:creator").unwrap();
        assert!(matches!(kind, PlatformKind::Patreon));
        assert_eq!(name, "creator");
    }
}
