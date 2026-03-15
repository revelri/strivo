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

    #[serde(default)]
    pub recording: RecordingConfig,

    #[serde(default)]
    pub auto_record_channels: Vec<AutoRecordEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchConfig {
    pub client_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeConfig {
    pub client_id: String,
    pub client_secret: String,
    pub cookies_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecordingConfig {
    #[serde(default)]
    pub transcode: bool,

    #[serde(default = "default_filename_template")]
    pub filename_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRecordEntry {
    pub platform: String,
    pub channel_id: String,
    pub channel_name: String,
}

fn default_recording_dir() -> PathBuf {
    directories::UserDirs::new()
        .map(|d| d.home_dir().join("Videos").join("StreaVo"))
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
            recording: RecordingConfig::default(),
            auto_record_channels: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "streavo")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn cache_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "streavo")
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".cache"))
    }

    pub fn state_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "streavo")
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
            let config = Self::default();
            config.save(Some(&path))?;
            return Ok(config);
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self, path: Option<&std::path::Path>) -> Result<()> {
        let path = path
            .map(|p| p.to_path_buf())
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
