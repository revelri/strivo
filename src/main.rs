#![allow(dead_code)]

mod app;
mod cli;
mod config;
mod monitor;
mod platform;
mod playback;
mod recording;
mod stream;
mod tui;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio::sync::{mpsc, RwLock};
use tracing_subscriber::EnvFilter;

use crate::app::AppEvent;
use crate::monitor::ChannelMonitor;
use crate::platform::Platform;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();

    // Initialize logging
    let state_dir = config::AppConfig::state_dir();
    std::fs::create_dir_all(&state_dir)?;
    let log_file = std::fs::File::create(state_dir.join("streavo.log"))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&args.log_level)),
        )
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    tracing::info!("StreaVo starting");

    // Load config
    let config = config::AppConfig::load(args.config.as_deref())?;
    tracing::info!(
        "Config loaded from {}",
        config::AppConfig::config_path().display()
    );

    // Create event channel (backend → TUI)
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();

    // Create recording command channel (TUI → recording manager)
    let (recording_tx, recording_rx) = mpsc::unbounded_channel();

    // Initialize platforms
    let mut platforms: Vec<Arc<RwLock<dyn Platform>>> = Vec::new();

    if let Some(ref twitch_config) = config.twitch {
        let mut twitch =
            platform::twitch::TwitchPlatform::new(twitch_config.client_id.clone());
        match twitch.authenticate().await {
            Ok(()) => {
                tracing::info!("Twitch authenticated");
                let tx = event_tx.clone();
                // Notify TUI of connection
                tokio::spawn(async move {
                    let _ = tx.send(AppEvent::Notification {
                        title: "Twitch Connected".to_string(),
                        body: "Successfully authenticated with Twitch".to_string(),
                    });
                });
            }
            Err(e) => {
                tracing::warn!("Twitch auth failed: {e}");
                let _ = event_tx.send(AppEvent::Error(format!("Twitch auth: {e}")));
            }
        }
        platforms.push(Arc::new(RwLock::new(twitch)));
    }

    if let Some(ref yt_config) = config.youtube {
        let mut youtube = platform::youtube::YouTubePlatform::new(
            yt_config.client_id.clone(),
            yt_config.client_secret.clone(),
            yt_config.cookies_path.clone(),
        );
        match youtube.authenticate().await {
            Ok(()) => {
                tracing::info!("YouTube authenticated");
            }
            Err(e) => {
                tracing::warn!("YouTube auth failed: {e}");
                let _ = event_tx.send(AppEvent::Error(format!("YouTube auth: {e}")));
            }
        }
        platforms.push(Arc::new(RwLock::new(youtube)));
    }

    // Spawn recording manager
    let rec_config = config.clone();
    let rec_tx = event_tx.clone();
    tokio::spawn(async move {
        recording::run_manager(rec_config, recording_rx, rec_tx).await;
    });

    // Spawn channel monitor
    if !platforms.is_empty() {
        let monitor = ChannelMonitor::new(
            platforms.clone(),
            config.clone(),
            event_tx.clone(),
            recording_tx.clone(),
        );
        tokio::spawn(async move {
            monitor.run().await;
        });
    }

    // Create app state
    let mut app_state = app::AppState::new(config.clone());
    app_state.twitch_connected = config.twitch.is_some();
    app_state.youtube_connected = config.youtube.is_some();

    // Run TUI (blocks until quit)
    tui::run(app_state, event_rx, recording_tx).await?;

    tracing::info!("StreaVo exiting");
    Ok(())
}
