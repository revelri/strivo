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
    // Internal sender for thumbnail download results
    let (thumb_tx, mut thumb_rx) = mpsc::unbounded_channel::<AppEvent>();

    loop {
        terminal.draw(|frame| layout::render(frame, app))?;

        // Poll crossterm events
        if let Some(event) = event::poll_event(FRAME_DURATION)? {
            if let Some(action) = app.handle_event(event) {
                handle_action(action, app, recording_tx, mpv).await;
            }
        }

        // Drain backend events
        while let Ok(event) = rx.try_recv() {
            // Check for channels updated to trigger thumbnail downloads
            if let AppEvent::ChannelsUpdated(ref channels) = event {
                spawn_thumbnail_downloads(channels, app, &thumb_tx);
            }
            if let Some(action) = app.handle_event(event) {
                handle_action(action, app, recording_tx, mpv).await;
            }
        }

        // Drain thumbnail events
        while let Ok(event) = thumb_rx.try_recv() {
            app.handle_event(event);
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

    for channel in channels {
        let Some(ref thumb_url) = channel.thumbnail_url else {
            continue;
        };

        // Skip if already cached
        if app.thumbnail_protocols.contains_key(&channel.id) {
            continue;
        }

        let channel_id = channel.id.clone();
        let url = thumb_url.clone();
        let tx = tx.clone();
        let cache_dir = cache_dir.clone();

        tokio::spawn(async move {
            if let Err(e) = download_thumbnail(&channel_id, &url, &cache_dir, &tx).await {
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
) -> Result<()> {
    std::fs::create_dir_all(cache_dir)?;

    let cache_path = cache_dir.join(format!("{channel_id}.jpg"));

    // Download
    let resp = reqwest::get(url).await?;
    let bytes = resp.bytes().await?;

    // Resize to reasonable thumbnail size
    let img = image::load_from_memory(&bytes)?;
    let resized = img.resize(440, 248, image::imageops::FilterType::Triangle);
    resized.save(&cache_path)?;

    let _ = tx.send(AppEvent::ThumbnailReady {
        channel_id: channel_id.to_string(),
        path: cache_path,
    });

    Ok(())
}

async fn handle_action(
    action: AppAction,
    app: &mut AppState,
    recording_tx: &mpsc::UnboundedSender<RecordingCommand>,
    mpv: &mut MpvController,
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

            let output_path = crate::recording::build_output_path(
                &app.config,
                &channel_name,
                platform,
            );
            let job = crate::recording::job::RecordingJob::new(
                channel_id.clone(),
                channel_name.clone(),
                platform,
                output_path,
                transcode,
            );
            app.register_recording(job);

            let _ = recording_tx.send(RecordingCommand::Start {
                channel_id,
                channel_name,
                platform,
                transcode,
                cookies_path,
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

            match resolver::resolve_stream_url(platform, &channel_name, cookies_path.as_deref())
                .await
            {
                Ok(info) => match mpv.play(&info.url).await {
                    Ok(()) => {
                        app.status_message = format!("Playing {channel_name} in mpv");
                        app.watching_channel = Some(channel_name);
                    }
                    Err(e) => {
                        app.status_message = format!("mpv error: {e}");
                    }
                },
                Err(e) => {
                    app.status_message = format!("Stream resolve error: {e}");
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
            let t = title.clone();
            let b = body.clone();
            tokio::task::spawn_blocking(move || {
                let _ = notify_rust::Notification::new()
                    .summary(&t)
                    .body(&b)
                    .appname("StreaVo")
                    .timeout(notify_rust::Timeout::Milliseconds(5000))
                    .show();
            });
        }
    }
}
