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
use crate::cli::{Command, ConfigAction, LogAction};
use crate::monitor::ChannelMonitor;
use crate::platform::Platform;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();

    // Handle subcommands that don't need TUI or logging
    if let Some(ref cmd) = args.command {
        return handle_command(cmd, args.config.as_deref()).await;
    }

    // Default: launch TUI
    run_tui(args).await
}

async fn handle_command(cmd: &Command, config_path: Option<&std::path::Path>) -> Result<()> {
    match cmd {
        Command::Config { action } => handle_config_command(action, config_path),
        Command::Log { action } => handle_log_command(action).await,
    }
}

fn handle_config_command(action: &ConfigAction, config_path: Option<&std::path::Path>) -> Result<()> {
    match action {
        ConfigAction::Path => {
            let path = config_path
                .map(|p| p.to_path_buf())
                .unwrap_or_else(config::AppConfig::config_path);
            println!("{}", path.display());
        }
        ConfigAction::List => {
            let cfg = config::AppConfig::load(config_path)?;
            println!("recording_dir = {:?}", cfg.recording_dir.display());
            println!("poll_interval_secs = {}", cfg.poll_interval_secs);
            println!("recording.transcode = {}", cfg.recording.transcode);
            println!("recording.filename_template = {:?}", cfg.recording.filename_template);
            if let Some(ref tw) = cfg.twitch {
                println!("twitch.client_id = {:?}", tw.client_id);
            } else {
                println!("twitch = <not configured>");
            }
            if let Some(ref yt) = cfg.youtube {
                println!("youtube.client_id = {:?}", yt.client_id);
                println!("youtube.client_secret = {:?}", yt.client_secret);
                if let Some(ref cp) = yt.cookies_path {
                    println!("youtube.cookies_path = {:?}", cp.display());
                }
            } else {
                println!("youtube = <not configured>");
            }
            if !cfg.auto_record_channels.is_empty() {
                println!();
                println!("auto_record_channels:");
                for entry in &cfg.auto_record_channels {
                    println!("  {} / {} ({})", entry.platform, entry.channel_name, entry.channel_id);
                }
            }
        }
        ConfigAction::Get { key } => {
            let cfg = config::AppConfig::load(config_path)?;
            let value = config_get(&cfg, key)?;
            println!("{value}");
        }
        ConfigAction::Set { key, value } => {
            let mut cfg = config::AppConfig::load(config_path)?;
            config_set(&mut cfg, key, value)?;
            cfg.save(config_path)?;
            println!("Set {key} = {value}");
        }
        ConfigAction::Reset => {
            let old = config::AppConfig::load(config_path)?;
            let mut cfg = config::AppConfig::default();
            // Preserve credentials
            cfg.twitch = old.twitch;
            cfg.youtube = old.youtube;
            cfg.auto_record_channels = old.auto_record_channels;
            cfg.save(config_path)?;
            println!("Config reset to defaults (credentials preserved)");
        }
    }
    Ok(())
}

fn config_get(cfg: &config::AppConfig, key: &str) -> Result<String> {
    match key {
        "recording_dir" => Ok(cfg.recording_dir.to_string_lossy().to_string()),
        "poll_interval" | "poll_interval_secs" => Ok(cfg.poll_interval_secs.to_string()),
        "transcode" | "recording.transcode" => Ok(cfg.recording.transcode.to_string()),
        "filename_template" | "recording.filename_template" => {
            Ok(cfg.recording.filename_template.clone())
        }
        "twitch.client_id" => cfg
            .twitch
            .as_ref()
            .map(|t| t.client_id.clone())
            .ok_or_else(|| anyhow::anyhow!("Twitch not configured")),
        "youtube.client_id" => cfg
            .youtube
            .as_ref()
            .map(|y| y.client_id.clone())
            .ok_or_else(|| anyhow::anyhow!("YouTube not configured")),
        "youtube.client_secret" => cfg
            .youtube
            .as_ref()
            .map(|y| y.client_secret.clone())
            .ok_or_else(|| anyhow::anyhow!("YouTube not configured")),
        "youtube.cookies_path" => cfg
            .youtube
            .as_ref()
            .and_then(|y| y.cookies_path.as_ref())
            .map(|p| p.to_string_lossy().to_string())
            .ok_or_else(|| anyhow::anyhow!("YouTube cookies path not set")),
        _ => Err(anyhow::anyhow!(
            "Unknown key: {key}\n\nValid keys:\n  \
             recording_dir, poll_interval, transcode, filename_template,\n  \
             twitch.client_id, youtube.client_id, youtube.client_secret, youtube.cookies_path"
        )),
    }
}

fn config_set(cfg: &mut config::AppConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "recording_dir" => {
            cfg.recording_dir = std::path::PathBuf::from(value);
        }
        "poll_interval" | "poll_interval_secs" => {
            cfg.poll_interval_secs = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid integer: {value}"))?;
            if cfg.poll_interval_secs < 15 {
                anyhow::bail!("Poll interval must be >= 15 seconds");
            }
        }
        "transcode" | "recording.transcode" => {
            cfg.recording.transcode = match value {
                "true" | "on" | "1" | "yes" => true,
                "false" | "off" | "0" | "no" => false,
                _ => anyhow::bail!("Invalid boolean: {value} (use true/false/on/off)"),
            };
        }
        "filename_template" | "recording.filename_template" => {
            cfg.recording.filename_template = value.to_string();
        }
        "twitch.client_id" => {
            if let Some(ref mut tw) = cfg.twitch {
                tw.client_id = value.to_string();
            } else {
                cfg.twitch = Some(config::TwitchConfig {
                    client_id: value.to_string(),
                });
            }
        }
        "youtube.client_id" => {
            if let Some(ref mut yt) = cfg.youtube {
                yt.client_id = value.to_string();
            } else {
                cfg.youtube = Some(config::YouTubeConfig {
                    client_id: value.to_string(),
                    client_secret: String::new(),
                    cookies_path: None,
                });
            }
        }
        "youtube.client_secret" => {
            if let Some(ref mut yt) = cfg.youtube {
                yt.client_secret = value.to_string();
            } else {
                cfg.youtube = Some(config::YouTubeConfig {
                    client_id: String::new(),
                    client_secret: value.to_string(),
                    cookies_path: None,
                });
            }
        }
        "youtube.cookies_path" => {
            if let Some(ref mut yt) = cfg.youtube {
                yt.cookies_path = Some(std::path::PathBuf::from(value));
            } else {
                cfg.youtube = Some(config::YouTubeConfig {
                    client_id: String::new(),
                    client_secret: String::new(),
                    cookies_path: Some(std::path::PathBuf::from(value)),
                });
            }
        }
        _ => {
            anyhow::bail!(
                "Unknown key: {key}\n\nValid keys:\n  \
                 recording_dir, poll_interval, transcode, filename_template,\n  \
                 twitch.client_id, youtube.client_id, youtube.client_secret, youtube.cookies_path"
            );
        }
    }
    Ok(())
}

async fn handle_log_command(action: &LogAction) -> Result<()> {
    let log_path = config::AppConfig::state_dir().join("streavo.log");

    match action {
        LogAction::Path => {
            println!("{}", log_path.display());
        }
        LogAction::Clear => {
            if log_path.exists() {
                std::fs::write(&log_path, "")?;
                println!("Log cleared: {}", log_path.display());
            } else {
                println!("No log file found at {}", log_path.display());
            }
        }
        LogAction::Tail { lines } => {
            tail_log(&log_path, *lines).await?;
        }
    }
    Ok(())
}

async fn tail_log(path: &std::path::Path, initial_lines: usize) -> Result<()> {
    use tokio::io::AsyncBufReadExt;

    if !path.exists() {
        println!("No log file at {}. Start StreaVo first.", path.display());
        return Ok(());
    }

    // Print last N lines
    let content = tokio::fs::read_to_string(path).await?;
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(initial_lines);
    for line in &all_lines[start..] {
        println!("{line}");
    }

    // Now tail (poll for new content)
    println!("--- tailing {} (Ctrl-C to stop) ---", path.display());

    let mut last_len = content.len() as u64;
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));

    loop {
        interval.tick().await;

        let meta = match tokio::fs::metadata(path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        let current_len = meta.len();
        if current_len <= last_len {
            if current_len < last_len {
                // File was truncated (e.g. log clear)
                last_len = 0;
                println!("--- log file truncated ---");
            }
            continue;
        }

        // Read new bytes
        let file = tokio::fs::File::open(path).await?;
        let mut reader = tokio::io::BufReader::new(file);

        // Seek to where we left off
        use tokio::io::AsyncSeekExt;
        reader.seek(std::io::SeekFrom::Start(last_len)).await?;

        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break;
            }
            print!("{line}");
        }

        last_len = current_len;
    }
}

async fn run_tui(args: cli::Args) -> Result<()> {
    // Initialize logging
    let state_dir = config::AppConfig::state_dir();
    std::fs::create_dir_all(&state_dir)?;

    let log_path = state_dir.join("streavo.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

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
