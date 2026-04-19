pub mod event;
pub mod layout;
pub mod theme;
pub mod widgets;

use crate::app::{AppAction, AppEvent, AppState, ActivePane, process_plugin_actions};
use crate::config::AppConfig;
use crate::playback::MpvController;
use crate::plugin::registry::PluginRegistry;
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
    mut registry: PluginRegistry,
    mut event_rx: mpsc::UnboundedReceiver<AppEvent>,
    recording_tx: mpsc::UnboundedSender<RecordingCommand>,
) -> Result<()> {
    app.recording_tx = Some(recording_tx.clone());
    let mut terminal = ratatui::init();
    let mut mpv = MpvController::new();

    let result =
        run_loop(&mut terminal, &mut app, &mut registry, &mut event_rx, &recording_tx, &mut mpv).await;

    // Cleanup
    registry.shutdown_all();
    mpv.quit().await.ok();
    ratatui::restore();
    result
}

async fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut AppState,
    registry: &mut PluginRegistry,
    rx: &mut mpsc::UnboundedReceiver<AppEvent>,
    recording_tx: &mpsc::UnboundedSender<RecordingCommand>,
    mpv: &mut MpvController,
) -> Result<()> {
    let (internal_tx, mut internal_rx) = mpsc::unbounded_channel::<AppEvent>();

    loop {
        terminal.draw(|frame| layout::render(frame, app, registry))?;

        // Poll crossterm events
        if let Some(evt) = event::poll_event(FRAME_DURATION)? {
            // Handle plugin key routing before app gets the event
            if let AppEvent::Key(ref key) = evt {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    // Plugin activation commands (global)
                    if !matches!(app.active_pane, ActivePane::Wizard | ActivePane::Plugin(_)) {
                        if let Some(pane_id) = registry.pane_for_command(key) {
                            app.active_pane = ActivePane::Plugin(pane_id);
                            registry.set_active_pane(Some(pane_id));
                            continue;
                        }
                    }
                    // Plugin pane key dispatch
                    if matches!(app.active_pane, ActivePane::Plugin(_)) {
                        let actions = registry.dispatch_key(*key, app);
                        if let Some(action) = process_plugin_actions(actions, app, registry) {
                            handle_action(action, app, recording_tx, mpv, &internal_tx).await;
                        }
                        continue;
                    }
                }
            }

            if let Some(action) = app.handle_event(evt) {
                handle_action(action, app, recording_tx, mpv, &internal_tx).await;
            }
        }

        // Drain backend events
        while let Ok(event) = rx.try_recv() {
            // Check for channels updated to trigger thumbnail downloads
            if let AppEvent::Daemon(crate::app::DaemonEvent::ChannelsUpdated(ref channels)) = event {
                spawn_thumbnail_downloads(channels, app, &internal_tx);
            }

            // Clone daemon event for plugin dispatch after app processes it
            let daemon_event_clone = if let AppEvent::Daemon(ref de) = event {
                Some(de.clone())
            } else {
                None
            };

            // Handle PluginEvent directly (no longer goes through app.handle_event)
            if let AppEvent::PluginEvent { plugin_name, event: pe } = event {
                let actions = registry.dispatch_plugin_event(plugin_name, pe);
                if let Some(action) = process_plugin_actions(actions, app, registry) {
                    handle_action(action, app, recording_tx, mpv, &internal_tx).await;
                }
                continue;
            }

            if let Some(action) = app.handle_event(event) {
                handle_action(action, app, recording_tx, mpv, &internal_tx).await;
            }

            // Dispatch daemon events to plugins AFTER app has processed them
            if let Some(de) = daemon_event_clone {
                let plugin_actions = registry.dispatch_event(&de, app);
                for action in plugin_actions {
                    handle_plugin_action(action, app, registry, &internal_tx);
                }
            }
        }

        // Drain internal async results
        while let Ok(event) = internal_rx.try_recv() {
            // Handle PluginEvent from internal channel too
            if let AppEvent::PluginEvent { plugin_name, event: pe } = event {
                let actions = registry.dispatch_plugin_event(plugin_name, pe);
                if let Some(action) = process_plugin_actions(actions, app, registry) {
                    handle_action(action, app, recording_tx, mpv, &internal_tx).await;
                }
                continue;
            }
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

    let resp = reqwest::get(url).await?;
    let bytes = resp.bytes().await?;

    let channel_id_owned = channel_id.to_string();
    let cache_path_clone = cache_path.clone();
    let decode_result = tokio::task::spawn_blocking(move || -> Result<Option<ratatui_image::protocol::StatefulProtocol>> {
        let img = image::load_from_memory(&bytes)?;
        let resized = img.resize(440, 248, image::imageops::FilterType::Triangle);
        resized.save(&cache_path_clone)?;

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

fn handle_plugin_action(
    action: crate::plugin::PluginAction,
    app: &mut AppState,
    registry: &mut PluginRegistry,
    tx: &mpsc::UnboundedSender<AppEvent>,
) {
    match action {
        crate::plugin::PluginAction::SetStatus(msg) => {
            app.status_message = msg;
        }
        crate::plugin::PluginAction::Notify { title, body } => {
            tokio::task::spawn_blocking(move || {
                let _ = notify_rust::Notification::new()
                    .summary(&title)
                    .body(&body)
                    .appname("StriVo")
                    .timeout(notify_rust::Timeout::Milliseconds(5000))
                    .show();
            });
        }
        crate::plugin::PluginAction::ActivatePane(pane_id) => {
            app.active_pane = ActivePane::Plugin(pane_id);
            registry.set_active_pane(Some(pane_id));
        }
        crate::plugin::PluginAction::NavigateBack => {
            app.active_pane = ActivePane::Sidebar;
            registry.set_active_pane(None);
        }
        crate::plugin::PluginAction::SpawnTask { plugin_name, future } => {
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = future.await;
                let _ = tx.send(AppEvent::PluginEvent {
                    plugin_name,
                    event: result,
                });
            });
        }
        crate::plugin::PluginAction::PlayFile(_path) => {
            // PlayFile from plugin actions is handled via process_plugin_actions -> AppAction
        }
        crate::plugin::PluginAction::UpdateConfig { plugin_name, config_update } => {
            match plugin_name {
                "crunchr" => {
                    if let Ok(cfg) = config_update.downcast::<crate::config::CrunchrConfig>() {
                        app.config.crunchr = *cfg;
                    }
                }
                "archiver" => {
                    if let Ok(cfg) = config_update.downcast::<crate::config::ArchiverConfig>() {
                        app.config.archiver = *cfg;
                    }
                }
                _ => {
                    tracing::warn!("Unknown plugin config update: {plugin_name}");
                }
            }
            if let Err(e) = app.config.save(None) {
                tracing::error!("Failed to save config after plugin update: {e}");
                app.status_message = format!("Config save failed: {e}");
            }
        }
    }
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
            from_start,
        } => {
            let cookies_path = match platform {
                crate::platform::PlatformKind::YouTube => app
                    .config
                    .youtube
                    .as_ref()
                    .and_then(|y| y.cookies_path.clone()),
                _ => None,
            };

            let stream_title = app
                .channels
                .iter()
                .find(|ch| ch.id == channel_id)
                .and_then(|ch| ch.stream_title.clone());

            let _ = recording_tx.send(RecordingCommand::Start {
                channel_id,
                channel_name,
                platform,
                transcode,
                cookies_path,
                stream_title,
                from_start,
                job_id: None,
            });
            app.status_message = if from_start {
                "Starting recording from stream start...".to_string()
            } else {
                "Starting recording...".to_string()
            };
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
                    .appname("StriVo")
                    .timeout(notify_rust::Timeout::Milliseconds(5000))
                    .show();
            });
        }
        AppAction::SpawnPluginTask { plugin_name, future } => {
            let tx = watch_tx.clone();
            tokio::spawn(async move {
                let result = future.await;
                let _ = tx.send(AppEvent::PluginEvent {
                    plugin_name,
                    event: result,
                });
            });
        }
        AppAction::ProbeMedia { job_id, path } => {
            let tx = watch_tx.clone();
            tokio::spawn(async move {
                match crate::media::probe_file(&path).await {
                    Ok(info) => {
                        let _ = tx.send(AppEvent::MediaProbed { job_id, info });
                    }
                    Err(e) => {
                        tracing::warn!("ffprobe failed for {}: {e}", path.display());
                    }
                }
            });
        }
        AppAction::OpenUrl { url } => {
            // Cross-platform: xdg-open (Linux), open (macOS), start (Windows).
            // Spawned detached so the TUI stays responsive regardless.
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else if cfg!(target_os = "windows") {
                "start"
            } else {
                "xdg-open"
            };
            let url_for_status = url.clone();
            tokio::task::spawn_blocking(move || {
                let _ = std::process::Command::new(opener)
                    .arg(&url)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            });
            app.status_message = format!("Opening {url_for_status}…");
        }
    }
}
