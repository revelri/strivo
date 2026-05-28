pub mod credentials;
pub mod import;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_recording_dir")]
    pub recording_dir: PathBuf,

    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,

    pub twitch: Option<TwitchConfig>,
    pub youtube: Option<YouTubeConfig>,
    pub patreon: Option<PatreonConfig>,

    #[serde(default)]
    pub recording: RecordingConfig,

    #[serde(default)]
    pub theme: ThemeRef,

    /// UI preferences — animation, a11y, verbosity. Everything optional.
    #[serde(default)]
    pub ui: UiConfig,

    #[serde(default)]
    pub auto_record_channels: Vec<AutoRecordEntry>,

    /// Named capture profiles (roadmap item 21), referenced by
    /// `AutoRecordEntry::profile`.
    #[serde(default)]
    pub capture_profiles: Vec<CaptureProfile>,

    /// Patreon creators whose new video posts should be auto-downloaded
    /// when the monitor sees them. Empty by default — the user has to
    /// opt in per creator (sidebar toggle); the monitor only refreshes
    /// the post cache for unlisted creators. Mirrors the
    /// auto_record_channels shape so the same UX patterns apply.
    #[serde(default)]
    pub auto_pull_creators: Vec<AutoPullEntry>,

    #[serde(default)]
    pub schedule: Vec<ScheduleEntry>,

    #[serde(default, alias = "sloptube")]
    pub crunchr: CrunchrConfig,

    #[serde(default)]
    pub archiver: ArchiverConfig,

    /// Web UI (`strivo serve`) settings. Generated lazily on first
    /// `serve` invocation; persisted so the API key survives restarts.
    #[serde(default)]
    pub web: WebConfig,

    /// Desktop notification preferences — what state changes the daemon
    /// should fire a notify-rust banner for.
    #[serde(default)]
    pub notifications: NotificationsConfig,

    /// Tracks the path this config was loaded from, so save() can use it
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
}

/// `[web]` config section. (Part 11.)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebConfig {
    /// API key for the `X-Api-Key` header on `/api/v1/*`. Persisted so
    /// repeated `strivo serve` invocations hand the same key to scripts.
    /// `None` means "generate on first run and save".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// HMAC secret used to sign browser-session cookies (W3). Persisted
    /// so cookies survive restarts. `None` means "generate on first
    /// session and save".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrunchrConfig {
    /// Whether the plugin is enabled (gates tandem auto-processing).
    #[serde(default)]
    pub enabled: bool,

    /// Whether the first-run config modal has been completed.
    #[serde(default)]
    pub configured: bool,

    #[serde(default = "default_crunchr_backend")]
    pub backend: String,

    /// Env var name for the transcription API key.
    /// Defaults: `OPENROUTER_API_KEY` for `voxtral-openrouter` (default backend),
    /// `MISTRAL_API_KEY` for `voxtral-api`.
    #[serde(default)]
    pub api_key_env: Option<String>,

    /// Base URL for self-hosted Voxtral (vLLM, RunPod, etc.).
    /// Only used when backend = "voxtral-local".
    #[serde(default)]
    pub endpoint: Option<String>,

    /// Preferred whisper model for CLI backend.
    #[serde(default)]
    pub whisper_model: Option<String>,

    /// Max seconds for whisper subprocess before timeout.
    #[serde(default = "default_whisper_timeout")]
    pub whisper_timeout_secs: u64,

    /// When true, request speaker diarization. Only honoured by backends that
    /// can produce speaker labels (`voxtral-api`, `whisperx-local`). Enables
    /// the Speaker Editor modal.
    #[serde(default)]
    pub diarize: bool,

    /// When true, mux the generated `.vtt` subtitles back into the recording's
    /// `.mkv` via `mkvmerge` after a transcription job finishes.
    #[serde(default = "default_embed_subs")]
    pub embed_subs: bool,

    #[serde(default)]
    pub analysis: CrunchrAnalysisConfig,

    /// Tandem mode: auto-trigger on RecordingFinished for these channels.
    /// Each entry is "Platform:channel_id" (e.g., "Twitch:123456").
    #[serde(default)]
    pub tandem_channels: Vec<String>,

    /// Tandem mode: auto-trigger for recordings from these playlists.
    #[serde(default)]
    pub tandem_playlists: Vec<String>,

    /// Soft budget for paid transcription/analysis backends, in cents
    /// per month. 0 disables the warning. Crunchr surfaces a status
    /// chip when spend ≥80% and refuses pre-submission of jobs that
    /// would tip spend over budget unless --force-spend is passed.
    /// (C2.) Backwards-compatible default keeps the warning off so
    /// existing configs upgrade silently.
    #[serde(default)]
    pub budget_cents_per_month: u64,

    /// Active preset name from the user's preset library (C1). When
    /// empty, the historical `backend` field path is used.
    #[serde(default)]
    pub active_preset: Option<String>,
}

impl Default for CrunchrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            configured: false,
            backend: default_crunchr_backend(),
            api_key_env: None,
            endpoint: None,
            whisper_model: None,
            whisper_timeout_secs: default_whisper_timeout(),
            diarize: false,
            embed_subs: default_embed_subs(),
            analysis: CrunchrAnalysisConfig::default(),
            tandem_channels: Vec::new(),
            tandem_playlists: Vec::new(),
            budget_cents_per_month: 0,
            active_preset: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrunchrAnalysisConfig {
    #[serde(default)]
    pub enabled: bool,

    /// Env var name for OpenRouter API key (e.g., "OPENROUTER_API_KEY").
    #[serde(default)]
    pub openrouter_api_key_env: Option<String>,

    /// OpenRouter model ID for analysis (e.g., "mistralai/mistral-7b-instruct").
    #[serde(default = "default_analysis_model")]
    pub model: String,
}

impl Default for CrunchrAnalysisConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            openrouter_api_key_env: None,
            model: default_analysis_model(),
        }
    }
}

fn default_crunchr_backend() -> String {
    "voxtral-openrouter".to_string()
}

fn default_whisper_timeout() -> u64 {
    7200
}

fn default_embed_subs() -> bool {
    true
}

fn default_analysis_model() -> String {
    "mistralai/mistral-7b-instruct".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiverConfig {
    /// Whether the plugin is enabled (gates tandem auto-processing).
    #[serde(default)]
    pub enabled: bool,

    /// Whether the first-run config modal has been completed.
    #[serde(default)]
    pub configured: bool,

    #[serde(default = "default_archive_dir")]
    pub archive_dir: PathBuf,
    #[serde(default = "default_archive_format")]
    pub format: String,
    #[serde(default = "default_concurrent_fragments")]
    pub concurrent_fragments: u32,
    #[serde(default)]
    pub rate_limit: String,

    /// Tandem mode: auto-trigger archiving for these channels.
    /// Each entry is "Platform:channel_id" (e.g., "Twitch:123456").
    #[serde(default)]
    pub tandem_channels: Vec<String>,

    /// Tandem mode: auto-trigger archiving for recordings from these playlists.
    #[serde(default)]
    pub tandem_playlists: Vec<String>,
}

impl Default for ArchiverConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            configured: false,
            archive_dir: default_archive_dir(),
            format: default_archive_format(),
            concurrent_fragments: default_concurrent_fragments(),
            rate_limit: String::new(),
            tandem_channels: Vec::new(),
            tandem_playlists: Vec::new(),
        }
    }
}

fn default_archive_dir() -> PathBuf {
    directories::UserDirs::new()
        .map(|d| d.home_dir().join("Videos/StriVo/Archives"))
        .unwrap_or_else(|| PathBuf::from("./archives"))
}
fn default_archive_format() -> String {
    "best".to_string()
}
fn default_concurrent_fragments() -> u32 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeConfig {
    pub client_id: String,
    pub client_secret: String,
    pub cookies_path: Option<PathBuf>,
    /// Public HTTPS URL of the WebSub (PubSubHubbub) callback served by
    /// `strivo serve` at `/yt-websub` (e.g. a `tailscale funnel` path). When
    /// set, the daemon subscribes each followed channel's upload feed to
    /// Google's hub so a new video / live broadcast triggers an immediate
    /// poll instead of waiting for the next interval. Unset = polling only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websub_callback_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatreonConfig {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default = "default_patreon_poll_interval")]
    pub poll_interval_secs: u64,
    /// Path to a Netscape cookies.txt for the patron's logged-in Patreon
    /// session. Required to list/download a pledged creator's video posts —
    /// the Patreon API doesn't expose them to patrons, so StriVo falls back
    /// to yt-dlp + these cookies (mirrors the YouTube cookies_path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookies_path: Option<PathBuf>,
}

fn default_patreon_poll_interval() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    #[serde(default)]
    pub transcode: bool,

    #[serde(default = "default_filename_template")]
    pub filename_template: String,

    /// Default format/bitrate selection. Per-channel `format` overrides this.
    #[serde(default)]
    pub format: RecordingFormat,

    /// On Twitch recordings, post-process the finished file to drop black
    /// segments left behind when streamlink suppresses ad breaks.
    #[serde(default)]
    pub auto_trim_ads: bool,

    /// Minimum contiguous black duration (seconds) to consider an ad break.
    /// Shorter blackouts (fades, transitions) are preserved.
    #[serde(default = "default_ad_min_secs")]
    pub ad_min_secs: f64,

    /// After a Twitch live recording ends, query the helix /videos endpoint
    /// for the just-finalized archive VOD and download it alongside the
    /// live capture. Saves to `<base>_vod.<ext>`. Useful because the live
    /// HLS pull misses the first ~5 minutes plus any ad-break gaps; the
    /// VOD captures the full broadcast.
    #[serde(default = "default_auto_vod_backfill")]
    pub auto_vod_backfill: bool,

    /// Seconds to wait after the live ends before polling for the VOD —
    /// Twitch takes a few minutes to finalize the archive.
    #[serde(default = "default_vod_backfill_delay_secs")]
    pub vod_backfill_delay_secs: u64,

    /// For Twitch + from_start recordings, attempt the live-from-start
    /// rewind path (helix → GQL → Usher /vod/v2) before falling back to
    /// the standard streamlink live-edge pull. When the streamer has
    /// "Always publish VODs" enabled, this lands at broadcast t=0
    /// instead of the ~5min HLS DVR window.
    #[serde(default = "default_twitch_live_from_start")]
    pub twitch_live_from_start: bool,
}

fn default_ad_min_secs() -> f64 {
    8.0
}

fn default_auto_vod_backfill() -> bool {
    true
}

fn default_vod_backfill_delay_secs() -> u64 {
    300
}

fn default_twitch_live_from_start() -> bool {
    true
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            transcode: false,
            filename_template: default_filename_template(),
            format: RecordingFormat::default(),
            auto_trim_ads: false,
            ad_min_secs: default_ad_min_secs(),
            auto_vod_backfill: default_auto_vod_backfill(),
            vod_backfill_delay_secs: default_vod_backfill_delay_secs(),
            twitch_live_from_start: default_twitch_live_from_start(),
        }
    }
}

/// Format / quality selection for a recording job.
///
/// All fields optional — `RecordingFormat::resolve(channel_override, global)` walks
/// channel → global → built-in defaults: `format = "best"`, copy-mux into MKV.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecordingFormat {
    /// yt-dlp `-f` selector, e.g. `"best"`, `"bestvideo[height<=1080]+bestaudio"`,
    /// `"bestaudio"`. Default: `"best"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Target video bitrate in kbps for transcode paths. Ignored when codec is `"copy"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_kbps: Option<u32>,

    /// Container: `"mkv"` (default — crash-resilient) or `"mp4"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,

    /// `"copy"` (default), `"h264_nvenc"`, `"libx264"`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,

    /// `"copy"` (default), `"aac"`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
}

impl RecordingFormat {
    /// Merge with precedence: per-channel override → global default → built-in defaults.
    /// Result has every field populated.
    pub fn resolved(channel: Option<&Self>, global: &Self) -> ResolvedFormat {
        let pick = |c: fn(&Self) -> Option<&str>| -> String {
            channel
                .and_then(|x| c(x))
                .or_else(|| c(global))
                .map(String::from)
                .unwrap_or_default()
        };
        let format = if let Some(s) = channel
            .and_then(|x| x.format.as_deref())
            .or(global.format.as_deref())
        {
            s.to_string()
        } else {
            "best".to_string()
        };
        let container = pick(|x| x.container.as_deref());
        let container = if container.is_empty() {
            "mkv".into()
        } else {
            container
        };
        let video_codec = pick(|x| x.video_codec.as_deref());
        let video_codec = if video_codec.is_empty() {
            "copy".into()
        } else {
            video_codec
        };
        let audio_codec = pick(|x| x.audio_codec.as_deref());
        let audio_codec = if audio_codec.is_empty() {
            "copy".into()
        } else {
            audio_codec
        };
        let bitrate_kbps = channel.and_then(|x| x.bitrate_kbps).or(global.bitrate_kbps);
        ResolvedFormat {
            format,
            bitrate_kbps,
            container,
            video_codec,
            audio_codec,
        }
    }
}

/// Fully-populated result of `RecordingFormat::resolved`.
#[derive(Debug, Clone)]
pub struct ResolvedFormat {
    pub format: String,
    pub bitrate_kbps: Option<u32>,
    pub container: String,
    pub video_codec: String,
    pub audio_codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRecordEntry {
    pub platform: String,
    pub channel_id: String,
    pub channel_name: String,

    /// Per-channel override of recording format/bitrate. Falls back to
    /// `recording.format` global when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<RecordingFormat>,

    /// Name of a `[[capture_profiles]]` entry to apply to this channel
    /// (roadmap item 21). Falls back to the global `[recording]` defaults
    /// when absent. Validated by `config_warnings`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// A named, reusable capture profile (roadmap item 21) — define recording
/// settings once ("1080p60+transcript", "audio-only") and attach to many
/// channels via `AutoRecordEntry::profile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureProfile {
    /// Unique profile name referenced by `AutoRecordEntry::profile`.
    pub name: String,

    /// Format/bitrate selection for channels using this profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<RecordingFormat>,

    /// Transcode the finished capture (overrides the global default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcode: Option<bool>,

    /// Capture audio only (no video).
    #[serde(default)]
    pub audio_only: bool,

    /// Request a transcript for captures using this profile.
    #[serde(default)]
    pub transcript: bool,

    /// Re-capture cutoff: once this many episodes are recorded for a channel
    /// using this profile, StriVo stops auto-capturing it. `None` = no cutoff
    /// (capture indefinitely). `Some(0)` is nonsensical and flagged by
    /// `config_warnings`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutoff_episodes: Option<u32>,
}

/// Per-creator opt-in to Patreon auto-download. Identifying by
/// campaign_id rather than the creator name because creators rename
/// themselves. creator_name is kept around purely for UI display when
/// the monitor hasn't yet refreshed pledged_creators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoPullEntry {
    pub campaign_id: String,
    #[serde(default)]
    pub creator_name: String,
}

/// Schedule-based recording entry.
/// Uses cron syntax for time-based recordings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub channel: String,
    pub cron: String,
    #[serde(default = "default_schedule_duration")]
    pub duration: String,
}

fn default_schedule_duration() -> String {
    "4h".to_string()
}

fn default_theme() -> String {
    "neon".to_string()
}

/// How `theme` is expressed in `config.toml`. Accepts either the legacy bare
/// string (`theme = "neon"`) or a rich table with per-slot overrides:
///
/// ```toml
/// [theme]
/// name = "tokyo-night"
/// [theme.colors]
/// primary = "#00E5FF"
/// [theme.ansi]
/// red = "#FF5555"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThemeRef {
    Named(String),
    Rich(ThemeSpec),
}

impl Default for ThemeRef {
    fn default() -> Self {
        ThemeRef::Named(default_theme())
    }
}

impl ThemeRef {
    pub fn name(&self) -> &str {
        match self {
            ThemeRef::Named(s) => s.as_str(),
            ThemeRef::Rich(s) => s.name.as_str(),
        }
    }

    /// Color slot overrides (keyed by semantic slot name: `bg`, `primary`, …).
    pub fn colors(&self) -> &BTreeMap<String, String> {
        static EMPTY: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
        match self {
            ThemeRef::Rich(s) => &s.colors,
            ThemeRef::Named(_) => EMPTY.get_or_init(BTreeMap::new),
        }
    }

    /// ANSI slot overrides (keyed by `red`, `blue`, …).
    pub fn ansi(&self) -> &BTreeMap<String, String> {
        static EMPTY: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
        match self {
            ThemeRef::Rich(s) => &s.ansi,
            ThemeRef::Named(_) => EMPTY.get_or_init(BTreeMap::new),
        }
    }

    /// Replace the theme name, preserving any overrides.
    pub fn set_name(&mut self, new_name: String) {
        match self {
            ThemeRef::Named(s) => *s = new_name,
            ThemeRef::Rich(s) => s.name = new_name,
        }
    }
}

/// Desktop notification preferences. Wired by the daemon's existing
/// notify-rust integration — each flag gates one class of banner.
/// Defaults err on the side of useful-but-not-noisy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    /// Master switch. When false the daemon skips every notify call.
    #[serde(default = "default_true")]
    pub desktop_enabled: bool,
    /// Banner when a tracked channel transitions offline → live.
    #[serde(default = "default_true")]
    pub on_go_live: bool,
    /// Banner when a recording finishes successfully (Twitch / YT live
    /// + Patreon VOD pull). Useful but can be noisy with bulk catalogs.
    #[serde(default = "default_true")]
    pub on_recording_finished: bool,
    /// Banner when a recording dies in the middle. Always default-on:
    /// silent failures are the worst class of bug in a PVR.
    #[serde(default = "default_true")]
    pub on_recording_failed: bool,
    /// Banner when the Twitch auto-VOD-backfill pull lands. Off by
    /// default because most users don't track it manually.
    #[serde(default)]
    pub on_vod_ready: bool,
}

fn default_true() -> bool { true }

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            desktop_enabled: true,
            on_go_live: true,
            on_recording_finished: true,
            on_recording_failed: true,
            on_vod_ready: false,
        }
    }
}

/// Motion / accessibility / verbosity preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiConfig {
    /// When true, animations snap to their end state instead of tweening.
    /// Mirrors `STRIVO_REDUCE_MOTION`; whichever is set wins.
    #[serde(default)]
    pub reduce_motion: bool,

    /// Verbose status-bar labels for screen readers / low-vision users.
    #[serde(default)]
    pub verbose_status: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeSpec {
    #[serde(default = "default_theme")]
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ansi: BTreeMap<String, String>,
}

fn default_recording_dir() -> PathBuf {
    directories::UserDirs::new()
        .map(|d| d.home_dir().join("Videos").join("StriVo"))
        .unwrap_or_else(|| PathBuf::from("./recordings"))
}

fn default_poll_interval() -> u64 {
    60
}

fn default_filename_template() -> String {
    "{channel}_{date}_{title}.mkv".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            recording_dir: default_recording_dir(),
            poll_interval_secs: default_poll_interval(),
            twitch: None,
            youtube: None,
            patreon: None,
            recording: RecordingConfig::default(),
            theme: ThemeRef::default(),
            ui: UiConfig::default(),
            auto_record_channels: Vec::new(),
            capture_profiles: Vec::new(),
            auto_pull_creators: Vec::new(),
            schedule: Vec::new(),
            crunchr: CrunchrConfig::default(),
            archiver: ArchiverConfig::default(),
            web: WebConfig::default(),
            notifications: NotificationsConfig::default(),
            config_path: None,
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "strivo")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// The capture profile attached to an auto-record channel, if any
    /// (roadmap item 21). `platform` is the `Display` form ("Twitch", …).
    pub fn capture_profile_for(&self, platform: &str, channel_id: &str) -> Option<&CaptureProfile> {
        let entry = self
            .auto_record_channels
            .iter()
            .find(|a| a.channel_id == channel_id && a.platform.eq_ignore_ascii_case(platform))?;
        let name = entry.profile.as_ref()?;
        self.capture_profiles.iter().find(|p| &p.name == name)
    }

    /// Effective transcode setting for an auto-record channel: the channel's
    /// capture profile overrides the global `[recording]` default.
    pub fn effective_transcode(&self, platform: &str, channel_id: &str) -> bool {
        self.capture_profile_for(platform, channel_id)
            .and_then(|p| p.transcode)
            .unwrap_or(self.recording.transcode)
    }

    /// Lint the config for pathological capture-profile / auto-record setups
    /// (roadmap item 21). Returns human-readable warnings; the daemon logs
    /// them at startup. Pure (no I/O) so it's unit-testable.
    pub fn config_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Duplicate profile names — the later one silently wins on lookup.
        let mut seen = std::collections::HashSet::new();
        for p in &self.capture_profiles {
            if !seen.insert(p.name.as_str()) {
                warnings.push(format!("capture profile '{}' is defined more than once", p.name));
            }
            // A zero cutoff would block all capture for attached channels.
            if p.cutoff_episodes == Some(0) {
                warnings.push(format!(
                    "capture profile '{}' has cutoff_episodes = 0; it will never record",
                    p.name
                ));
            }
        }

        // auto_record entries referencing a profile that doesn't exist.
        for ch in &self.auto_record_channels {
            if let Some(ref prof) = ch.profile {
                if !self.capture_profiles.iter().any(|p| &p.name == prof) {
                    warnings.push(format!(
                        "channel '{}' references unknown capture profile '{}'",
                        ch.channel_name, prof
                    ));
                }
            }
        }

        // Perpetual re-record: a channel that is BOTH auto-recorded (every live
        // session) AND on a cron schedule will double-capture the same content.
        for ch in &self.auto_record_channels {
            if self
                .schedule
                .iter()
                .any(|s| s.channel.eq_ignore_ascii_case(&ch.channel_name))
            {
                warnings.push(format!(
                    "channel '{}' is both auto-recorded and scheduled — likely perpetual re-capture",
                    ch.channel_name
                ));
            }
        }

        warnings
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn cache_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "strivo")
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".cache"))
    }

    pub fn data_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "strivo")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".data"))
    }

    pub fn state_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "strivo")
            .map(|d| d.state_dir().unwrap_or(d.data_dir()).to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".state"))
    }

    pub fn load(path: Option<&std::path::Path>) -> Result<Self> {
        let path = path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::config_path);

        if !path.exists() {
            let mut config = Self::default();
            config.config_path = Some(path.clone());
            config.save(Some(&path))?;
            return Ok(config);
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;

        match toml::from_str::<Self>(&contents) {
            Ok(mut config) => {
                config.config_path = Some(path);
                Ok(config)
            }
            Err(parse_err) => {
                // Live file is corrupt. Try `.backup` (previous known-good),
                // else quarantine the bad file and fall back to defaults so
                // the user is never stranded without a runnable config.
                let backup = backup_path(&path);
                if backup.exists() {
                    if let Ok(bcontents) = std::fs::read_to_string(&backup) {
                        if let Ok(mut config) = toml::from_str::<Self>(&bcontents) {
                            let _ = quarantine(&path);
                            let _ = std::fs::copy(&backup, &path);
                            config.config_path = Some(path);
                            return Ok(config);
                        }
                    }
                }
                let _ = quarantine(&path);
                let mut config = Self::default();
                config.config_path = Some(path.clone());
                let _ = config.save(Some(&path));
                eprintln!(
                    "config: {} was malformed ({}); fell back to defaults",
                    path.display(),
                    parse_err
                );
                Ok(config)
            }
        }
    }

    /// Reset all user-authored values to their defaults, **preserving
    /// credentials** (`twitch`, `youtube`, `patreon`) so the user is
    /// not logged out by a reset. Schedule entries, auto-record
    /// channels, recording defaults, UI prefs, plugin config — all
    /// revert. The caller is responsible for `save()`.
    ///
    /// M2.1.c — defaults-as-preset model. Defaults live in code
    /// (`Default::default()`); the on-disk file is the user's overlay.
    /// "Reset" means "write the empty overlay back."
    pub fn reset_to_defaults(&mut self) {
        let mut fresh = Self::default();
        fresh.config_path = self.config_path.clone();
        fresh.twitch = self.twitch.take();
        fresh.youtube = self.youtube.take();
        fresh.patreon = self.patreon.take();
        *self = fresh;
    }

    pub fn save(&self, path: Option<&std::path::Path>) -> Result<()> {
        let path = path
            .map(|p| p.to_path_buf())
            .or_else(|| self.config_path.clone())
            .unwrap_or_else(Self::config_path);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory {}", parent.display())
            })?;
        }

        // Rotate the prior live file into `.backup` before overwriting so
        // a crash mid-write can never wedge the user into a broken state.
        if path.exists() {
            let backup = backup_path(&path);
            let _ = std::fs::copy(&path, &backup);
        }

        let contents = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(&path, contents)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;
        Ok(())
    }
}

fn backup_path(path: &std::path::Path) -> PathBuf {
    let mut s = path.to_path_buf().into_os_string();
    s.push(".backup");
    PathBuf::from(s)
}

fn quarantine(path: &std::path::Path) -> std::io::Result<()> {
    let mut s = path.to_path_buf().into_os_string();
    s.push(".corrupt");
    std::fs::rename(path, PathBuf::from(s))
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    fn arc(name: &str, profile: Option<&str>) -> AutoRecordEntry {
        AutoRecordEntry {
            platform: "Twitch".into(),
            channel_id: "1".into(),
            channel_name: name.into(),
            format: None,
            profile: profile.map(String::from),
        }
    }

    #[test]
    fn warns_on_unknown_profile_and_zero_cutoff_and_dupes() {
        let mut cfg = AppConfig::default();
        cfg.capture_profiles = vec![
            CaptureProfile {
                name: "hq".into(),
                format: None,
                transcode: None,
                audio_only: false,
                transcript: true,
                cutoff_episodes: Some(0),
            },
            CaptureProfile {
                name: "hq".into(), // duplicate
                format: None,
                transcode: None,
                audio_only: false,
                transcript: false,
                cutoff_episodes: None,
            },
        ];
        cfg.auto_record_channels = vec![arc("Foo", Some("nonexistent"))];
        let w = cfg.config_warnings();
        assert!(w.iter().any(|s| s.contains("defined more than once")), "{w:?}");
        assert!(w.iter().any(|s| s.contains("never record")), "{w:?}");
        assert!(w.iter().any(|s| s.contains("unknown capture profile")), "{w:?}");
    }

    #[test]
    fn warns_on_auto_record_plus_schedule() {
        let mut cfg = AppConfig::default();
        cfg.auto_record_channels = vec![arc("LilAggy", None)];
        cfg.schedule = vec![ScheduleEntry {
            channel: "lilaggy".into(),
            cron: "0 20 * * *".into(),
            duration: "4h".into(),
        }];
        let w = cfg.config_warnings();
        assert!(w.iter().any(|s| s.contains("perpetual re-capture")), "{w:?}");
    }

    #[test]
    fn effective_transcode_honours_profile_override() {
        let mut cfg = AppConfig::default();
        cfg.recording.transcode = false; // global default off
        cfg.capture_profiles = vec![CaptureProfile {
            name: "hq".into(),
            format: None,
            transcode: Some(true), // profile turns it on
            audio_only: false,
            transcript: false,
            cutoff_episodes: None,
        }];
        cfg.auto_record_channels = vec![AutoRecordEntry {
            platform: "Twitch".into(),
            channel_id: "42".into(),
            channel_name: "Foo".into(),
            format: None,
            profile: Some("hq".into()),
        }];
        // Channel with the profile → override wins.
        assert!(cfg.effective_transcode("Twitch", "42"));
        // Unknown channel → global default.
        assert!(!cfg.effective_transcode("Twitch", "999"));
    }

    #[test]
    fn clean_config_has_no_warnings() {
        let mut cfg = AppConfig::default();
        cfg.capture_profiles = vec![CaptureProfile {
            name: "hq".into(),
            format: None,
            transcode: Some(true),
            audio_only: false,
            transcript: true,
            cutoff_episodes: Some(5),
        }];
        cfg.auto_record_channels = vec![arc("Foo", Some("hq"))];
        assert!(cfg.config_warnings().is_empty());
    }
}
