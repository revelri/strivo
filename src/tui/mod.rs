pub mod event;
pub mod layout;
pub mod theme;
pub mod widgets;

use crate::app::{AppAction, AppEvent, AppState};
use crate::config::AppConfig;
use crate::playback::MpvController;
use crate::recording::RecordingCommand;
use crate::stream::resolver;
use anyhow::Result;
use ratatui_image::picker::Picker;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

const FRAME_DURATION: Duration = Duration::from_millis(33); // ~30fps

pub async fn run(
    mut app: AppState,
    mut event_rx: mpsc::UnboundedReceiver<AppEvent>,
    recording_tx: mpsc::UnboundedSender<RecordingCommand>,
) -> Result<()> {
    app.recording_tx = Some(recording_tx.clone());
    let mut terminal = ratatui::init();
    let mut mpv = MpvController::new();

    let result =
        run_loop(&mut terminal, &mut app, &mut event_rx, &recording_tx, &mut mpv).await;

    // Cleanup
    mpv.quit().await.ok();
    ratatui::restore();
    result
}

async fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut AppState,
    rx: &mut mpsc::UnboundedReceiver<AppEvent>,
    recording_tx: &mpsc::UnboundedSender<RecordingCommand>,
    mpv: &mut MpvController,
) -> Result<()> {
    // Internal sender for async results (thumbnails, watch resolution, etc.)
    let (internal_tx, mut internal_rx) = mpsc::unbounded_channel::<AppEvent>();

    loop {
        terminal.draw(|frame| layout::render(frame, app))?;

        // Poll crossterm events
        if let Some(event) = event::poll_event(FRAME_DURATION)? {
            if let Some(action) = app.handle_event(event) {
                handle_action(action, app, recording_tx, mpv, &internal_tx).await;
            }
        }

        // Drain backend events
        while let Ok(event) = rx.try_recv() {
            // Check for channels updated to trigger thumbnail downloads
            if let AppEvent::ChannelsUpdated(ref channels) = event {
                spawn_thumbnail_downloads(channels, app, &internal_tx);
            }
            if let Some(action) = app.handle_event(event) {
                handle_action(action, app, recording_tx, mpv, &internal_tx).await;
            }
        }

        // Drain internal async results (thumbnails, watch resolution, etc.)
        while let Ok(event) = internal_rx.try_recv() {
            if let Some(action) = app.handle_event(event) {
                handle_action(action, app, recording_tx, mpv, &internal_tx).await;
            }
        }

        // Clear watching_channel if mpv has exited
        if app.watching_channel.is_some() && !mpv.is_running() {
            app.watching_channel = None;
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn spawn_thumbnail_downloads(
    channels: &[crate::platform::ChannelEntry],
    app: &AppState,
    tx: &mpsc::UnboundedSender<AppEvent>,
) {
    let cache_dir = AppConfig::cache_dir().join("thumbnails");
    let picker = app.picker.clone();

    for channel in channels {
        let Some(ref thumb_url) = channel.thumbnail_url else {
            continue;
        };

        // Skip if already downloaded (even if protocol loading failed)
        if app.thumbnail_cache.contains_key(&channel.id) {
            continue;
        }

        let channel_id = channel.id.clone();
        let url = thumb_url.clone();
        let tx = tx.clone();
        let cache_dir = cache_dir.clone();
        let picker = picker.clone();

        tokio::spawn(async move {
            if let Err(e) = download_thumbnail(&channel_id, &url, &cache_dir, &tx, picker).await {
                tracing::debug!("Thumbnail download failed for {channel_id}: {e}");
            }
        });
    }
}

async fn download_thumbnail(
    channel_id: &str,
    url: &str,
    cache_dir: &PathBuf,
    tx: &mpsc::UnboundedSender<AppEvent>,
    picker: Option<Picker>,
) -> Result<()> {
    std::fs::create_dir_all(cache_dir)?;

    let cache_path = cache_dir.join(format!("{channel_id}.jpg"));

    // Download
    let resp = reqwest::get(url).await?;
    let bytes = resp.bytes().await?;

    // Decode, resize, and create protocol off the event loop
    let channel_id_owned = channel_id.to_string();
    let cache_path_clone = cache_path.clone();
    let decode_result = tokio::task::spawn_blocking(move || -> Result<Option<ratatui_image::protocol::StatefulProtocol>> {
        let img = image::load_from_memory(&bytes)?;
        let resized = img.resize(440, 248, image::imageops::FilterType::Triangle);
        resized.save(&cache_path_clone)?;

        // Create protocol if picker is available
        if let Some(picker) = picker {
            let proto = picker.new_resize_protocol(resized.into());
            Ok(Some(proto))
        } else {
            Ok(None)
        }
    }).await??;

    let _ = tx.send(AppEvent::ThumbnailReady {
        channel_id: channel_id.to_string(),
        path: cache_path,
    });

    if let Some(protocol) = decode_result {
        let _ = tx.send(AppEvent::ThumbnailDecoded {
            channel_id: channel_id_owned,
            protocol,
        });
    }

    Ok(())
}

async fn handle_action(
    action: AppAction,
    app: &mut AppState,
    recording_tx: &mpsc::UnboundedSender<RecordingCommand>,
    mpv: &mut MpvController,
    watch_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    match action {
        AppAction::StartRecording {
            channel_id,
            channel_name,
            platform,
            transcode,
        } => {
            let cookies_path = match platform {
                crate::platform::PlatformKind::YouTube => app
                    .config
                    .youtube
                    .as_ref()
                    .and_then(|y| y.cookies_path.clone()),
                _ => None,
            };

            // Look up stream_title from the channel
            let stream_title = app
                .channels
                .iter()
                .find(|ch| ch.id == channel_id)
                .and_then(|ch| ch.stream_title.clone());

            // Recording manager is the single source of truth for RecordingJob.
            // The TUI registers the job when RecordingStarted arrives.
            let _ = recording_tx.send(RecordingCommand::Start {
                channel_id,
                channel_name,
                platform,
                transcode,
                cookies_path,
                stream_title,
            });
            app.status_message = "Starting recording...".to_string();
        }
        AppAction::Watch {
            channel_name,
            platform,
        } => {
            app.status_message = format!("Resolving stream for {channel_name}...");

            let cookies_path = match platform {
                crate::platform::PlatformKind::YouTube => app
                    .config
                    .youtube
                    .as_ref()
                    .and_then(|y| y.cookies_path.clone()),
                _ => None,
            };

            // Spawn resolution to avoid blocking the UI
            let watch_tx = watch_tx.clone();
            let ch_name = channel_name.clone();
            tokio::spawn(async move {
                match resolver::resolve_stream_url(platform, &ch_name, cookies_path.as_deref())
                    .await
                {
                    Ok(info) => {
                        let _ = watch_tx.send(AppEvent::WatchResolved {
                            channel_name: ch_name,
                            stream_url: info.url,
                        });
                    }
                    Err(e) => {
                        let _ = watch_tx.send(AppEvent::WatchFailed {
                            error: format!("{e}"),
                        });
                    }
                }
            });
        }
        AppAction::LaunchMpv { channel_name, url } => {
            match mpv.play(&url).await {
                Ok(()) => {
                    app.status_message = format!("Playing {channel_name} in mpv");
                }
                Err(e) => {
                    app.status_message = format!("mpv error: {e}");
                    app.watching_channel = None;
                }
            }
        }
        AppAction::PlayFile { path } => match mpv.play_file(&path).await {
            Ok(()) => {
                app.status_message = format!("Playing {}", path.display());
            }
            Err(e) => {
                app.status_message = format!("mpv error: {e}");
            }
        },
        AppAction::Notify { title, body } => {
            tokio::task::spawn_blocking(move || {
                let _ = notify_rust::Notification::new()
                    .summary(&title)
                    .body(&body)
                    .appname("StreaVo")
                    .timeout(notify_rust::Timeout::Milliseconds(5000))
                    .show();
            });
        }
    }
}
