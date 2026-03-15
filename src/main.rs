mod app;
mod cli;
mod config;
mod monitor;
mod platform;
mod playback;
mod recording;
mod stream;
mod tui;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

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
    tracing::info!("Config loaded from {}", config::AppConfig::config_path().display());

    // Create app state
    let app_state = app::AppState::new(config);

    // Create event channel for backend → TUI communication
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();

    // Run TUI
    tui::run(app_state, rx).await?;

    tracing::info!("StreaVo exiting");
    Ok(())
}
