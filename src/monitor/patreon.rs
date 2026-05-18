use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::{AppEvent, DaemonEvent};
use crate::config::AppConfig;
use crate::platform::patreon::PatreonClient;
use crate::platform::PlatformKind;
use crate::recording::RecordingCommand;

pub struct PatreonMonitor {
    client: PatreonClient,
    config: AppConfig,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    recording_tx: mpsc::UnboundedSender<RecordingCommand>,
    cancel: CancellationToken,
    /// Last check time per campaign
    last_checked: HashMap<String, DateTime<Utc>>,
    /// State file for persisting last_checked
    state_path: PathBuf,
}

impl PatreonMonitor {
    pub fn new(
        client: PatreonClient,
        config: AppConfig,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        recording_tx: mpsc::UnboundedSender<RecordingCommand>,
        cancel: CancellationToken,
    ) -> Self {
        let state_path = AppConfig::state_dir().join("patreon_state.json");
        let last_checked = load_state(&state_path).unwrap_or_default();

        Self {
            client,
            config,
            event_tx,
            recording_tx,
            cancel,
            last_checked,
            state_path,
        }
    }

    pub async fn run(mut self) {
        let poll_interval = self
            .config
            .patreon
            .as_ref()
            .map(|p| p.poll_interval_secs)
            .unwrap_or(300);
        let interval = std::time::Duration::from_secs(poll_interval.max(60));

        // Wait for auth
        if !self.client.is_authenticated().await {
            tracing::info!("Patreon monitor waiting for authentication...");
            return;
        }

        // Initial poll
        if let Err(e) = self.poll().await {
            tracing::error!("Patreon initial poll error: {e}");
        }

        let mut timer = tokio::time::interval(interval);
        timer.tick().await; // consume immediate tick

        loop {
            tokio::select! {
                _ = timer.tick() => {
                    if let Err(e) = self.poll().await {
                        tracing::error!("Patreon poll error: {e}");
                    }
                }
                _ = self.cancel.cancelled() => {
                    tracing::info!("Patreon monitor shutting down");
                    break;
                }
            }
        }
    }

    async fn poll(&mut self) -> anyhow::Result<()> {
        let creators = self.client.fetch_pledged_creators().await?;
        tracing::debug!("Patreon: found {} pledged creators", creators.len());

        for creator in &creators {
            let since = self.last_checked.get(&creator.campaign_id).copied();
            let posts = self
                .client
                .fetch_posts(&creator.campaign_id, since.as_ref())
                .await?;

            for post in &posts {
                let Some(ref embed_url) = post.embed_url else {
                    continue;
                };

                tracing::info!("Patreon new video post: {} - {}", creator.name, post.title);

                // Send notification
                let _ = self
                    .event_tx
                    .send(AppEvent::Daemon(DaemonEvent::PatreonPostFound {
                        creator_name: creator.name.clone(),
                        post_title: post.title.clone(),
                    }));
                let _ = self.event_tx.send(AppEvent::notification(
                    format!("Patreon: {}", creator.name),
                    post.title.clone(),
                ));

                // Trigger download
                let output_path = crate::recording::build_output_path(
                    &self.config,
                    &creator.name,
                    PlatformKind::Patreon,
                    Some(&post.title),
                );
                let _ = self.recording_tx.send(RecordingCommand::DownloadVod {
                    url: embed_url.clone(),
                    channel_name: creator.name.clone(),
                    platform: PlatformKind::Patreon,
                    output_path,
                    cookies_path: None,
                    post_title: Some(post.title.clone()),
                });
            }

            // Update last_checked
            self.last_checked
                .insert(creator.campaign_id.clone(), Utc::now());
        }

        // Persist state
        save_state(&self.state_path, &self.last_checked).ok();

        Ok(())
    }
}

fn load_state(path: &PathBuf) -> Option<HashMap<String, DateTime<Utc>>> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_state(path: &PathBuf, state: &HashMap<String, DateTime<Utc>>) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(path, content)?;
    Ok(())
}
