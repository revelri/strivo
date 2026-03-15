use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::app::AppEvent;
use crate::config::AppConfig;
use crate::platform::{ChannelEntry, Platform, PlatformKind};
use crate::recording::RecordingCommand;

pub struct ChannelMonitor {
    platforms: Vec<Arc<RwLock<dyn Platform>>>,
    config: AppConfig,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    recording_tx: mpsc::UnboundedSender<RecordingCommand>,
    /// Track which channels were previously live for went-live/went-offline detection
    prev_live: HashMap<String, bool>,
    /// Auto-record channels we've already triggered for (avoid duplicate starts)
    auto_recorded: HashMap<String, bool>,
}

impl ChannelMonitor {
    pub fn new(
        platforms: Vec<Arc<RwLock<dyn Platform>>>,
        config: AppConfig,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        recording_tx: mpsc::UnboundedSender<RecordingCommand>,
    ) -> Self {
        Self {
            platforms,
            config,
            event_tx,
            recording_tx,
            prev_live: HashMap::new(),
            auto_recorded: HashMap::new(),
        }
    }

    pub async fn run(mut self) {
        let poll_interval =
            std::time::Duration::from_secs(self.config.poll_interval_secs.max(15));
        let mut interval = tokio::time::interval(poll_interval);

        // Initial delay to let auth complete
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        loop {
            interval.tick().await;

            if let Err(e) = self.poll_all().await {
                tracing::error!("Monitor poll error: {e}");
                let _ = self.event_tx.send(AppEvent::Error(format!("Poll error: {e}")));
            }
        }
    }

    async fn poll_all(&mut self) -> Result<()> {
        let mut all_channels: Vec<ChannelEntry> = Vec::new();

        for platform in &self.platforms {
            let plat = platform.read().await;
            match plat.fetch_followed_channels().await {
                Ok(mut channels) => {
                    // Check live status
                    let ids: Vec<String> = channels.iter().map(|c| c.id.clone()).collect();
                    match plat.check_live_status(&ids).await {
                        Ok(live_channels) => {
                            let live_map: HashMap<String, ChannelEntry> = live_channels
                                .into_iter()
                                .map(|c| (c.id.clone(), c))
                                .collect();

                            for ch in &mut channels {
                                if let Some(live) = live_map.get(&ch.id) {
                                    ch.is_live = true;
                                    ch.stream_title = live.stream_title.clone();
                                    ch.game_or_category = live.game_or_category.clone();
                                    ch.viewer_count = live.viewer_count;
                                    ch.started_at = live.started_at;
                                    ch.thumbnail_url = live.thumbnail_url.clone();
                                }

                                // Apply auto-record setting from config
                                ch.auto_record = self.config.auto_record_channels.iter().any(|a| {
                                    a.channel_id == ch.id
                                        && a.platform == plat.kind().to_string()
                                });
                            }
                        }
                        Err(e) => {
                            tracing::warn!("{}: live status check failed: {e}", plat.kind());
                        }
                    }

                    // Detect went-live / went-offline transitions
                    for ch in &channels {
                        let was_live = self.prev_live.get(&ch.id).copied().unwrap_or(false);
                        if ch.is_live && !was_live {
                            let _ = self.event_tx.send(AppEvent::ChannelWentLive(ch.clone()));

                            // Auto-record trigger
                            if ch.auto_record && !self.auto_recorded.get(&ch.id).copied().unwrap_or(false) {
                                self.auto_recorded.insert(ch.id.clone(), true);
                                let cookies_path = self.get_cookies_path(ch.platform);
                                let _ = self.recording_tx.send(RecordingCommand::Start {
                                    channel_id: ch.id.clone(),
                                    channel_name: ch.name.clone(),
                                    platform: ch.platform,
                                    transcode: self.config.recording.transcode,
                                    cookies_path,
                                });
                            }
                        } else if !ch.is_live && was_live {
                            let _ = self.event_tx.send(AppEvent::ChannelWentOffline(ch.clone()));
                            self.auto_recorded.remove(&ch.id);
                        }
                        self.prev_live.insert(ch.id.clone(), ch.is_live);
                    }

                    all_channels.extend(channels);
                }
                Err(e) => {
                    tracing::warn!("{}: fetch channels failed: {e}", plat.kind());
                }
            }
        }

        // Sort: live first, then alphabetical
        all_channels.sort_by(|a, b| {
            a.platform
                .to_string()
                .cmp(&b.platform.to_string())
                .then(b.is_live.cmp(&a.is_live))
                .then(a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))
        });

        let _ = self.event_tx.send(AppEvent::ChannelsUpdated(all_channels));

        // Fetch thumbnails for live channels
        self.fetch_thumbnails().await;

        Ok(())
    }

    async fn fetch_thumbnails(&self) {
        // Thumbnail fetching handled via events — send thumbnail URLs for the TUI to cache
        // This is done in the poll_all by including thumbnail_url in ChannelEntry
    }

    fn get_cookies_path(&self, platform: PlatformKind) -> Option<PathBuf> {
        match platform {
            PlatformKind::YouTube => self
                .config
                .youtube
                .as_ref()
                .and_then(|y| y.cookies_path.clone()),
            _ => None,
        }
    }
}
