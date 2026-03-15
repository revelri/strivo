pub mod twitch;
pub mod youtube;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlatformKind {
    Twitch,
    YouTube,
}

impl std::fmt::Display for PlatformKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformKind::Twitch => write!(f, "Twitch"),
            PlatformKind::YouTube => write!(f, "YouTube"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChannelEntry {
    pub id: String,
    pub platform: PlatformKind,
    pub name: String,
    pub display_name: String,
    pub is_live: bool,
    pub stream_title: Option<String>,
    pub game_or_category: Option<String>,
    pub viewer_count: Option<u64>,
    pub started_at: Option<DateTime<Utc>>,
    pub thumbnail_url: Option<String>,
    pub auto_record: bool,
}

#[allow(dead_code)]
#[async_trait::async_trait]
pub trait Platform: Send + Sync {
    fn kind(&self) -> PlatformKind;
    async fn authenticate(&mut self) -> anyhow::Result<()>;
    async fn fetch_followed_channels(&self) -> anyhow::Result<Vec<ChannelEntry>>;
    async fn check_live_status(&self, channel_ids: &[String]) -> anyhow::Result<Vec<ChannelEntry>>;
    async fn refresh_token(&mut self) -> anyhow::Result<()>;
}
