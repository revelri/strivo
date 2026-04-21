pub mod credentials;

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

    #[serde(default)]
    pub schedule: Vec<ScheduleEntry>,

    #[serde(default, alias = "sloptube")]
    pub crunchr: CrunchrConfig,

    #[serde(default)]
    pub archiver: ArchiverConfig,

    /// Tracks the path this config was loaded from, so save() can use it
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
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

    /// Env var name for the transcription API key (e.g., "MISTRAL_API_KEY").
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

    #[serde(default)]
    pub analysis: CrunchrAnalysisConfig,

    /// Tandem mode: auto-trigger on RecordingFinished for these channels.
    /// Each entry is "Platform:channel_id" (e.g., "Twitch:123456").
    #[serde(default)]
    pub tandem_channels: Vec<String>,

    /// Tandem mode: auto-trigger for recordings from these playlists.
    #[serde(default)]
    pub tandem_playlists: Vec<String>,
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
            analysis: CrunchrAnalysisConfig::default(),
            tandem_channels: Vec::new(),
            tandem_playlists: Vec::new(),
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
    "whisper-cli".to_string()
}

fn default_whisper_timeout() -> u64 {
    7200
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
fn default_archive_format() -> String { "best".to_string() }
fn default_concurrent_fragments() -> u32 { 4 }

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatreonConfig {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default = "default_patreon_poll_interval")]
    pub poll_interval_secs: u64,
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
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            transcode: false,
            filename_template: default_filename_template(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRecordEntry {
    pub platform: String,
    pub channel_id: String,
    pub channel_name: String,
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
            schedule: Vec::new(),
            crunchr: CrunchrConfig::default(),
            archiver: ArchiverConfig::default(),
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
            .map(|d| {
                d.state_dir()
                    .unwrap_or(d.data_dir())
                    .to_path_buf()
            })
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

    pub fn save(&self, path: Option<&std::path::Path>) -> Result<()> {
        let path = path
            .map(|p| p.to_path_buf())
            .or_else(|| self.config_path.clone())
            .unwrap_or_else(Self::config_path);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory {}", parent.display()))?;
        }

        // Rotate the prior live file into `.backup` before overwriting so
        // a crash mid-write can never wedge the user into a broken state.
        if path.exists() {
            let backup = backup_path(&path);
            let _ = std::fs::copy(&path, &backup);
        }

        let contents = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
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
