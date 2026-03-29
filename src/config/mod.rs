pub mod credentials;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
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

    #[serde(default = "default_theme")]
    pub theme: String,

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
}

impl Default for CrunchrConfig {
    fn default() -> Self {
        Self {
            backend: default_crunchr_backend(),
            api_key_env: None,
            endpoint: None,
            whisper_model: None,
            whisper_timeout_secs: default_whisper_timeout(),
            analysis: CrunchrAnalysisConfig::default(),
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
    #[serde(default = "default_archive_dir")]
    pub archive_dir: PathBuf,
    #[serde(default = "default_archive_format")]
    pub format: String,
    #[serde(default = "default_concurrent_fragments")]
    pub concurrent_fragments: u32,
    #[serde(default)]
    pub rate_limit: String,
    #[serde(default)]
    pub auto_archive: bool,
}

impl Default for ArchiverConfig {
    fn default() -> Self {
        Self {
            archive_dir: default_archive_dir(),
            format: default_archive_format(),
            concurrent_fragments: default_concurrent_fragments(),
            rate_limit: String::new(),
            auto_archive: false,
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
            theme: default_theme(),
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
        let mut config: Self = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?;
        config.config_path = Some(path);
        Ok(config)
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

        let contents = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        std::fs::write(&path, contents)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;
        Ok(())
    }
}
