//! Cookie-source resolution.

use std::path::PathBuf;

use crate::config::AppConfig;
use crate::platform::PlatformKind;

/// Where the engine should source cookies for this recording.
///
/// Picked once by the caller to capture *its own context*, not the
/// stream's platform. The Patreon monitor already holds a live HTTP
/// session and picks [`CookieSource::Inherit`]; the daemon translator
/// for a webui-initiated pull picks [`CookieSource::FromConfig`]; tests
/// or one-off CLI tooling can pin an exact file with
/// [`CookieSource::Explicit`].
#[derive(Debug, Clone)]
pub enum CookieSource {
    /// Caller already holds a session; engine skips `--cookies`.
    Inherit,
    /// Look up the per-platform cookies path from [`AppConfig`].
    FromConfig,
    /// Use this exact path.
    Explicit(PathBuf),
}

impl CookieSource {
    /// Resolve to a concrete path (or `None` to mean "no cookies file").
    pub fn resolve(&self, config: &AppConfig, platform: PlatformKind) -> Option<PathBuf> {
        match self {
            Self::Inherit => None,
            Self::FromConfig => match platform {
                PlatformKind::YouTube => {
                    config.youtube.as_ref().and_then(|y| y.cookies_path.clone())
                }
                PlatformKind::Patreon => {
                    config.patreon.as_ref().and_then(|p| p.cookies_path.clone())
                }
                _ => None,
            },
            Self::Explicit(p) => Some(p.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_yt_cookies(p: &str) -> AppConfig {
        let mut c = AppConfig::default();
        c.youtube = Some(crate::config::YouTubeConfig {
            client_id: String::new(),
            client_secret: String::new(),
            cookies_path: Some(PathBuf::from(p)),
            websub_callback_url: None,
        });
        c
    }

    fn cfg_with_patreon_cookies(p: &str) -> AppConfig {
        let mut c = AppConfig::default();
        c.patreon = Some(crate::config::PatreonConfig {
            client_id: String::new(),
            client_secret: String::new(),
            poll_interval_secs: 300,
            cookies_path: Some(PathBuf::from(p)),
        });
        c
    }

    #[test]
    fn inherit_returns_none_on_every_platform() {
        let c = cfg_with_yt_cookies("/cookies/yt.txt");
        for p in [
            PlatformKind::Twitch,
            PlatformKind::YouTube,
            PlatformKind::Patreon,
        ] {
            assert_eq!(CookieSource::Inherit.resolve(&c, p), None);
        }
    }

    #[test]
    fn from_config_picks_youtube_path_for_youtube() {
        let c = cfg_with_yt_cookies("/cookies/yt.txt");
        assert_eq!(
            CookieSource::FromConfig.resolve(&c, PlatformKind::YouTube),
            Some(PathBuf::from("/cookies/yt.txt"))
        );
    }

    #[test]
    fn from_config_picks_patreon_path_for_patreon() {
        let c = cfg_with_patreon_cookies("/cookies/p.txt");
        assert_eq!(
            CookieSource::FromConfig.resolve(&c, PlatformKind::Patreon),
            Some(PathBuf::from("/cookies/p.txt"))
        );
    }

    #[test]
    fn from_config_returns_none_for_twitch() {
        let c = cfg_with_yt_cookies("/cookies/yt.txt");
        assert_eq!(
            CookieSource::FromConfig.resolve(&c, PlatformKind::Twitch),
            None
        );
    }

    #[test]
    fn explicit_overrides_platform() {
        let c = AppConfig::default();
        let s = CookieSource::Explicit(PathBuf::from("/x.txt"));
        assert_eq!(
            s.resolve(&c, PlatformKind::Twitch),
            Some(PathBuf::from("/x.txt"))
        );
    }
}
