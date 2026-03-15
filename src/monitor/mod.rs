use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::app::AppEvent;
use crate::config::AppConfig;
use crate::platform::{ChannelEntry, Platform, PlatformKind};
use crate::recording::RecordingCommand;

pub struct ChannelMonitor {
    platforms: Vec<Arc<RwLock<dyn Platform>>>,
    config: AppConfig,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    recording_tx: mpsc::UnboundedSender<RecordingCommand>,
    cancel: CancellationToken,
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
        cancel: CancellationToken,
    ) -> Self {
        Self {
            platforms,
            config,
            event_tx,
            recording_tx,
            cancel,
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
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.poll_all().await {
                        tracing::error!("Monitor poll error: {e}");
                        let _ = self.event_tx.send(AppEvent::Error(format!("Poll error: {e}")));
                    }
                }
                _ = self.cancel.cancelled() => {
                    tracing::info!("Monitor shutting down");
                    break;
                }
            }
        }
    }

    async fn poll_all(&mut self) -> Result<()> {
        let mut all_channels: Vec<ChannelEntry> = Vec::new();

        for platform in &self.platforms {
            // Clone necessary data before releasing the lock to avoid holding
            // it across network calls
            let (kind, channels_result) = {
                let plat = platform.read().await;
                let kind = plat.kind();
                let result = plat.fetch_followed_channels().await;
                (kind, result)
            };

            match channels_result {
                Ok(mut channels) => {
                    // Check live status without holding the platform lock
                    let ids: Vec<String> = channels.iter().map(|c| c.id.clone()).collect();
                    let live_result = {
                        let plat = platform.read().await;
                        plat.check_live_status(&ids).await
                    };

                    match live_result {
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

                                // Check auto-record from the channel data directly
                                // (reflects fresh config state from TUI saves)
                                ch.auto_record = self.config.auto_record_channels.iter().any(|a| {
                                    a.channel_id == ch.id
                                        && a.platform == kind.to_string()
                                });
                            }
                        }
                        Err(e) => {
                            tracing::warn!("{kind}: live status check failed: {e}");
                        }
                    }

                    // Detect went-live / went-offline transitions
                    for ch in &channels {
                        let was_live = self.prev_live.get(&ch.id).copied().unwrap_or(false);
                        if ch.is_live && !was_live {
                            let _ = self.event_tx.send(AppEvent::ChannelWentLive(ch.clone()));

                            // Auto-record trigger: use ch.auto_record from fresh data
                            if ch.auto_record && !self.auto_recorded.get(&ch.id).copied().unwrap_or(false) {
                                self.auto_recorded.insert(ch.id.clone(), true);
                                let cookies_path = self.get_cookies_path(ch.platform);
                                let _ = self.recording_tx.send(RecordingCommand::Start {
                                    channel_id: ch.id.clone(),
                                    channel_name: ch.name.clone(),
                                    platform: ch.platform,
                                    transcode: self.config.recording.transcode,
                                    cookies_path,
                                    stream_title: ch.stream_title.clone(),
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
                    tracing::warn!("{kind}: fetch channels failed: {e}");
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

        Ok(())
    }

    fn get_cookies_path(&self, platform: PlatformKind) -> Option<PathBuf> {
        // Reload config to get fresh auto_record and cookies settings
        let cfg = AppConfig::load(self.config.config_path.as_deref()).unwrap_or(self.config.clone());
        match platform {
            PlatformKind::YouTube => cfg
                .youtube
                .as_ref()
                .and_then(|y| y.cookies_path.clone()),
            _ => None,
        }
    }
}
