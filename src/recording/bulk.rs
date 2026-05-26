//! Per-channel bulk back-catalog downloader (task #71).
//!
//! Wraps the catalog pull engine (`catalog::run_pull`) in a daemon-side
//! manager that the TUI/webui can start and stop per channel. Each running
//! pull owns a `CancellationToken`; `Stop` cancels it and the engine breaks
//! between VODs. Progress is forwarded to the UI as `DaemonEvent::BulkProgress`.

use std::collections::HashMap;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::{AppEvent, DaemonEvent};
use crate::config::{AppConfig, RecordingFormat};
use crate::platform::{Platform, PlatformKind, VodEntry};
use crate::recording::catalog::{self, CatalogProgress, CatalogPullOptions};
use crate::recording::persist::PersistDb;

/// Commands the manager accepts. The TUI sends these over IPC; the daemon
/// forwards them to the manager's channel.
#[derive(Debug, Clone)]
pub enum BulkCommand {
    Start {
        channel_id: String,
        channel_name: String,
        platform: PlatformKind,
        /// Optional YouTube playlist scope (task #73). When set, only that
        /// playlist's items are pulled instead of the whole channel.
        playlist_id: Option<String>,
    },
    Stop {
        channel_id: String,
    },
    /// Fetch a YouTube channel's playlists and emit them as
    /// DaemonEvent::PlaylistList for the scope picker (task #73).
    ListPlaylists {
        channel_id: String,
    },
}

/// Spawn the bulk-download manager. Returns the command sender; the daemon
/// keeps it and forwards `ClientMessage::BulkDownload` onto it.
pub fn spawn(
    config: AppConfig,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) -> mpsc::UnboundedSender<BulkCommand> {
    let (tx, mut rx) = mpsc::unbounded_channel::<BulkCommand>();
    // Internal clone so spawned pulls can self-deregister (Stop) without
    // moving the public sender into the manager task.
    let internal_tx = tx.clone();
    tokio::spawn(async move {
        // channel_id -> cancellation handle for the in-flight pull.
        let mut active: HashMap<String, CancellationToken> = HashMap::new();
        while let Some(cmd) = rx.recv().await {
            match cmd {
                BulkCommand::Start {
                    channel_id,
                    channel_name,
                    platform,
                    playlist_id,
                } => {
                    if active.contains_key(&channel_id) {
                        tracing::info!(channel = %channel_name, "bulk-dl already running");
                        continue;
                    }
                    let cancel = CancellationToken::new();
                    active.insert(channel_id.clone(), cancel.clone());
                    let cfg = config.clone();
                    let etx = event_tx.clone();
                    let done_tx = internal_tx.clone();
                    tokio::spawn(async move {
                        run_channel_pull(
                            &cfg,
                            &channel_id,
                            &channel_name,
                            platform,
                            playlist_id.as_deref(),
                            cancel,
                            &etx,
                        )
                        .await;
                        // Self-deregister so a later Start can re-run.
                        let _ = done_tx.send(BulkCommand::Stop {
                            channel_id: channel_id.clone(),
                        });
                    });
                }
                BulkCommand::Stop { channel_id } => {
                    if let Some(cancel) = active.remove(&channel_id) {
                        cancel.cancel();
                    }
                }
                BulkCommand::ListPlaylists { channel_id } => {
                    let cfg = config.clone();
                    let etx = event_tx.clone();
                    tokio::spawn(async move {
                        let playlists = fetch_playlists(&cfg, &channel_id).await.unwrap_or_else(|e| {
                            tracing::warn!("bulk-dl: fetch_playlists failed: {e:#}");
                            Vec::new()
                        });
                        let _ = etx.send(AppEvent::Daemon(DaemonEvent::PlaylistList {
                            channel_id,
                            playlists,
                        }));
                    });
                }
            }
        }
    });
    tx
}

/// Resolve a channel's catalog and run the pull, emitting BulkProgress.
#[allow(clippy::too_many_arguments)]
async fn run_channel_pull(
    config: &AppConfig,
    channel_id: &str,
    channel_name: &str,
    platform: PlatformKind,
    playlist_id: Option<&str>,
    cancel: CancellationToken,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    let emit = |done: usize, total: usize, active: bool| {
        let _ = event_tx.send(AppEvent::Daemon(DaemonEvent::BulkProgress {
            channel_id: channel_id.to_string(),
            done,
            total,
            active,
        }));
    };

    emit(0, 0, true);

    let vods = match resolve_vods(config, channel_id, platform, playlist_id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(channel = %channel_name, "bulk-dl resolve failed: {e:#}");
            let _ = event_tx.send(AppEvent::error(format!("Bulk DL {channel_name}: {e}")));
            emit(0, 0, false);
            return;
        }
    };
    let total = vods.len();
    if total == 0 {
        let _ = event_tx.send(AppEvent::notification(
            format!("Bulk DL: {channel_name}"),
            "No new catalog items".to_string(),
        ));
        emit(0, 0, false);
        return;
    }

    // Forward catalog progress as a running done/total counter.
    let (ptx, mut prx) = mpsc::unbounded_channel::<CatalogProgress>();
    let etx2 = event_tx.clone();
    let cid = channel_id.to_string();
    tokio::spawn(async move {
        let mut done = 0usize;
        while let Some(ev) = prx.recv().await {
            match ev {
                CatalogProgress::Finished { .. }
                | CatalogProgress::Skipped { .. }
                | CatalogProgress::Failed { .. } => {
                    done += 1;
                    let _ = etx2.send(AppEvent::Daemon(DaemonEvent::BulkProgress {
                        channel_id: cid.clone(),
                        done,
                        total,
                        active: true,
                    }));
                }
                _ => {}
            }
        }
    });

    let cookies_path = if matches!(platform, PlatformKind::YouTube) {
        config.youtube.as_ref().and_then(|y| y.cookies_path.clone())
    } else {
        None
    };
    let chan_override = config
        .auto_record_channels
        .iter()
        .find(|c| c.channel_id == channel_id && c.platform == platform.to_string())
        .and_then(|c| c.format.clone());
    let resolved = RecordingFormat::resolved(chan_override.as_ref(), &config.recording.format);

    let opts = CatalogPullOptions {
        root: config.recording_dir.clone(),
        channel_name: channel_name.to_string(),
        format: resolved,
        cookies_path,
        force: false,
        crunchr_auto: config.crunchr.enabled,
    };

    let db_path = AppConfig::data_dir().join("jobs.db");
    let db = match PersistDb::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            let _ = event_tx.send(AppEvent::error(format!("Bulk DL db: {e}")));
            emit(0, total, false);
            return;
        }
    };

    match catalog::run_pull(&db, vods, &opts, Some(ptx), Some(&cancel)).await {
        Ok(report) => {
            let _ = event_tx.send(AppEvent::notification(
                format!("Bulk DL: {channel_name}"),
                format!(
                    "downloaded {} · skipped {} · failed {}",
                    report.downloaded,
                    report.skipped_existing,
                    report.failed.len()
                ),
            ));
        }
        Err(e) => {
            let _ = event_tx.send(AppEvent::error(format!("Bulk DL {channel_name}: {e}")));
        }
    }
    emit(total, total, false);
}

/// Fetch a YouTube channel's playlists for the scope picker (task #73).
async fn fetch_playlists(
    config: &AppConfig,
    channel_id: &str,
) -> anyhow::Result<Vec<crate::platform::PlaylistInfo>> {
    use anyhow::Context;
    let cfg = config
        .youtube
        .clone()
        .context("youtube section missing in config")?;
    let yt = crate::platform::youtube::YouTubePlatform::new(
        cfg.client_id,
        cfg.client_secret,
        cfg.cookies_path.clone(),
    );
    yt.load_stored_tokens().await.context("youtube auth")?;
    yt.fetch_playlists(channel_id).await
}

/// Resolve a channel's back-catalog VODs. Mirrors the `pull` CLI path.
async fn resolve_vods(
    config: &AppConfig,
    channel_id: &str,
    platform: PlatformKind,
    playlist_id: Option<&str>,
) -> anyhow::Result<Vec<VodEntry>> {
    use anyhow::Context;
    match platform {
        PlatformKind::YouTube => {
            let cfg = config
                .youtube
                .clone()
                .context("youtube section missing in config")?;
            let yt = crate::platform::youtube::YouTubePlatform::new(
                cfg.client_id,
                cfg.client_secret,
                cfg.cookies_path.clone(),
            );
            yt.load_stored_tokens().await.context("youtube auth")?;
            // Playlist scope (task #73): pull only that playlist's items.
            if let Some(pl) = playlist_id {
                yt.fetch_playlist_items(pl, channel_id, None, None).await
            } else {
                yt.fetch_channel_vods(channel_id, None, None).await
            }
        }
        PlatformKind::Twitch => {
            let cfg = config
                .twitch
                .clone()
                .context("twitch section missing in config")?;
            let tw = crate::platform::twitch::TwitchPlatform::new(cfg.client_id, cfg.client_secret);
            tw.load_stored_tokens().await.context("twitch auth")?;
            tw.fetch_channel_vods(channel_id, None, None).await
        }
        PlatformKind::Patreon => {
            let cfg = config
                .patreon
                .clone()
                .context("patreon section missing in config")?;
            let pt = crate::platform::patreon::PatreonClient::new(cfg.client_id, cfg.client_secret);
            pt.load_stored_tokens().await.context("patreon auth")?;
            pt.fetch_channel_vods(channel_id, None, None).await
        }
    }
}
