use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::events::DaemonEvent;
use crate::config::AppConfig;
use crate::platform::patreon::PatreonClient;
use crate::platform::PlatformKind;
use crate::recording::RecordingCommand;

pub struct PatreonMonitor {
    client: PatreonClient,
    config: AppConfig,
    event_tx: mpsc::UnboundedSender<DaemonEvent>,
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
        event_tx: mpsc::UnboundedSender<DaemonEvent>,
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

        // Accumulate the full snapshot for the TUI (task #69). creators
        // become sidebar rows; posts populate the per-creator right-pane
        // list and are cached by campaign_id.
        let mut creator_entries = Vec::with_capacity(creators.len());
        let mut all_posts = Vec::new();

        for creator in &creators {
            creator_entries.push(creator.to_channel_entry());

            // Fetch the recent window unconditionally (since=None) so the
            // right pane always shows the latest posts, not just those
            // newer than our last check. "New" detection for auto-pull is
            // done below by comparing published_at against last_checked.
            let last = self.last_checked.get(&creator.campaign_id).copied();
            // The Patreon API returns no posts for campaigns you only patron;
            // when cookies are configured, list them via yt-dlp instead.
            let cookies = self.config.patreon.as_ref().and_then(|p| p.cookies_path.clone());
            let posts = match (cookies.as_deref(), creator.vanity.as_deref()) {
                (Some(cp), Some(vanity)) => {
                    match self
                        .client
                        .fetch_posts_ytdlp(&creator.campaign_id, vanity, Some(cp), 15)
                        .await
                    {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::warn!("Patreon yt-dlp post fetch for {vanity} failed: {e:#}");
                            Vec::new()
                        }
                    }
                }
                _ => self.client.fetch_posts(&creator.campaign_id, None).await.unwrap_or_default(),
            };

            // Per-creator opt-in to auto-download (task #70). Creators with
            // no auto_pull_creators entry get a notification only; the user
            // manually triggers pull from the right pane (task #69).
            let auto_pull = self
                .config
                .auto_pull_creators
                .iter()
                .any(|e| e.campaign_id == creator.campaign_id);

            for post in &posts {
                let Some(ref embed_url) = post.embed_url else {
                    continue;
                };

                // Is this post new since we last polled this creator?
                let is_new = match (last, post.published_at.parse::<DateTime<Utc>>()) {
                    (Some(last), Ok(published)) => published > last,
                    // No prior watermark (first poll) or unparseable date:
                    // treat as not-new so a fresh auth doesn't trigger a
                    // back-catalog stampede. Auto-pull only fires on posts
                    // that appear after the first successful poll.
                    _ => false,
                };

                if is_new {
                    let _ = self.event_tx.send(DaemonEvent::PatreonPostFound {
                        creator_name: creator.name.clone(),
                        post_title: post.title.clone(),
                    });
                    let _ = self.event_tx.send(DaemonEvent::Notification {
                        title: format!("Patreon: {}", creator.name),
                        body: post.title.clone(),
                    });
                }

                if is_new && auto_pull {
                    tracing::info!(
                        "Patreon auto-pull: {} - {}",
                        creator.name,
                        post.title
                    );
                    // Monitor already holds the Patreon HTTP client's
                    // live session; `CookieSource::Inherit` skips the
                    // `--cookies` flag and lets the adapter carry the auth.
                    let spec = crate::intents::DownloadVodSpec {
                        url: embed_url.clone(),
                        channel_name: creator.name.clone(),
                        platform: PlatformKind::Patreon,
                        post_title: Some(post.title.clone()),
                        cookies: crate::intents::CookieSource::Inherit,
                        output_policy: crate::intents::OutputPathPolicy::Fresh,
                    };
                    let _ = self
                        .recording_tx
                        .send(crate::intents::download_vod(spec, &self.config));
                }
            }

            all_posts.extend(posts);

            // Update last_checked
            self.last_checked
                .insert(creator.campaign_id.clone(), Utc::now());
        }

        // Push the snapshot to the host (SPA).
        let _ = self.event_tx.send(DaemonEvent::PatreonState {
            creators: creator_entries,
            posts: all_posts,
        });

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
