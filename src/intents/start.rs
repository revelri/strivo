//! Translate a [`StartSpec`] into a [`RecordingCommand::Start`].

use crate::config::AppConfig;
use crate::intents::spec::StartSpec;
use crate::recording::RecordingCommand;

/// Build the engine-ready `Start` command.
///
/// Applies config-derived defaults:
/// - cookies via `spec.cookies.resolve(config, spec.platform)`
/// - transcode via `spec.transcode_override` or
///   `config.effective_transcode(platform, channel_id)`
///
/// Everything else is plumbed through verbatim. Pure: no IO, no
/// channels, no async.
pub fn start_recording(spec: StartSpec, config: &AppConfig) -> RecordingCommand {
    let cookies_path = spec.cookies.resolve(config, spec.platform);
    let transcode = spec
        .transcode_override
        .unwrap_or_else(|| config.effective_transcode(&spec.platform.to_string(), &spec.channel_id));

    RecordingCommand::Start {
        channel_id: spec.channel_id,
        channel_name: spec.channel_name,
        display_name: spec.display_name,
        platform: spec.platform,
        transcode,
        cookies_path,
        stream_title: spec.stream_title,
        from_start: spec.from_start,
        job_id: spec.job_id,
        thumbnail_url: spec.thumbnail_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intents::cookies::CookieSource;
    use crate::platform::PlatformKind;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn base_spec(platform: PlatformKind) -> StartSpec {
        StartSpec {
            channel_id: "abc".into(),
            channel_name: "abc".into(),
            display_name: Some("Display".into()),
            platform,
            stream_title: Some("title".into()),
            thumbnail_url: Some("https://t".into()),
            from_start: false,
            job_id: None,
            transcode_override: None,
            cookies: CookieSource::Inherit,
        }
    }

    #[test]
    fn transcode_override_wins_over_config() {
        let cfg = AppConfig::default();
        let mut spec = base_spec(PlatformKind::Twitch);
        spec.transcode_override = Some(true);
        match start_recording(spec, &cfg) {
            RecordingCommand::Start { transcode, .. } => assert!(transcode),
            _ => panic!("expected Start"),
        }
    }

    #[test]
    fn transcode_none_defers_to_config() {
        let mut cfg = AppConfig::default();
        cfg.recording.transcode = true;
        let spec = base_spec(PlatformKind::Twitch);
        match start_recording(spec, &cfg) {
            RecordingCommand::Start { transcode, .. } => assert!(transcode),
            _ => panic!("expected Start"),
        }
    }

    #[test]
    fn cookies_resolved_from_source_with_platform_context() {
        let mut cfg = AppConfig::default();
        cfg.youtube = Some(crate::config::YouTubeConfig {
            client_id: String::new(),
            client_secret: String::new(),
            cookies_path: Some(PathBuf::from("/y.txt")),
            websub_callback_url: None,
        });
        let mut spec = base_spec(PlatformKind::YouTube);
        spec.cookies = CookieSource::FromConfig;
        match start_recording(spec, &cfg) {
            RecordingCommand::Start { cookies_path, .. } => {
                assert_eq!(cookies_path, Some(PathBuf::from("/y.txt")));
            }
            _ => panic!("expected Start"),
        }
    }

    #[test]
    fn plumbed_fields_round_trip_unchanged() {
        let cfg = AppConfig::default();
        let id = Uuid::new_v4();
        let mut spec = base_spec(PlatformKind::YouTube);
        spec.from_start = true;
        spec.job_id = Some(id);
        match start_recording(spec, &cfg) {
            RecordingCommand::Start {
                from_start,
                job_id,
                thumbnail_url,
                display_name,
                stream_title,
                ..
            } => {
                assert!(from_start);
                assert_eq!(job_id, Some(id));
                assert_eq!(thumbnail_url, Some("https://t".into()));
                assert_eq!(display_name, Some("Display".into()));
                assert_eq!(stream_title, Some("title".into()));
            }
            _ => panic!("expected Start"),
        }
    }
}
