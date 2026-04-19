// Crate-wide clippy allows — these flag style preferences, not bugs, and
// touching them now would be pure churn. Re-enable per-module when rewriting
// the relevant code.
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::type_complexity)]

use strivo::{app, cli, config, daemon, ipc, monitor, platform, plugin, recording, search, tui};

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::app::AppEvent;
use crate::cli::{Command, ConfigAction, LogAction};
use crate::tui::theme::Theme;
use crate::monitor::ChannelMonitor;
use crate::platform::Platform;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();

    if let Some(ref cmd) = args.command {
        return handle_command(cmd, args.config.as_deref()).await;
    }

    // Initialize theme from config (needed before TUI rendering)
    let theme_config = config::AppConfig::load(args.config.as_deref()).ok();
    let theme_name = theme_config
        .as_ref()
        .map(|c| c.theme.as_str())
        .unwrap_or("neon");
    Theme::init(theme_name);

    // Default: try connecting to daemon, fall back to standalone TUI
    if ipc::is_daemon_running() {
        run_client(args).await
    } else {
        run_tui(args).await
    }
}

async fn handle_command(cmd: &Command, config_path: Option<&std::path::Path>) -> Result<()> {
    match cmd {
        Command::Daemon => daemon::run().await,
        Command::Enable => handle_enable().await,
        Command::Disable => handle_disable().await,
        Command::Status => handle_status(),
        Command::Config { action } => handle_config_command(action, config_path),
        Command::Log { action } => handle_log_command(action).await,
        Command::Search { query } => handle_search(query, config_path),
    }
}

fn handle_status() -> Result<()> {
    if ipc::is_daemon_running() {
        println!("StriVo daemon is running");
        let pid_path = ipc::pid_path();
        if let Ok(pid) = std::fs::read_to_string(&pid_path) {
            println!("PID: {}", pid.trim());
        }
        println!("Socket: {}", ipc::socket_path().display());
    } else {
        println!("StriVo daemon is not running");
        println!("Start with: strivo daemon");
        println!("Or enable as service: strivo enable");
    }
    Ok(())
}

async fn handle_enable() -> Result<()> {
    let exe = std::env::current_exe()?;
    let unit_content = format!(
        "[Unit]\n\
         Description=StriVo Live Stream PVR Daemon\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} daemon\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe.display()
    );

    let systemd_dir = dirs_home().join(".config/systemd/user");
    std::fs::create_dir_all(&systemd_dir)?;
    let unit_path = systemd_dir.join("strivo.service");
    std::fs::write(&unit_path, unit_content)?;
    println!("Wrote {}", unit_path.display());

    let status = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()?;
    if !status.success() {
        anyhow::bail!("systemctl daemon-reload failed");
    }

    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "strivo.service"])
        .status()?;
    if !status.success() {
        anyhow::bail!("systemctl enable --now failed");
    }

    println!("StriVo daemon enabled and started");
    Ok(())
}

async fn handle_disable() -> Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "strivo.service"])
        .status()?;
    if !status.success() {
        eprintln!("Warning: systemctl disable --now may have failed");
    }

    let unit_path = dirs_home().join(".config/systemd/user/strivo.service");
    if unit_path.exists() {
        std::fs::remove_file(&unit_path)?;
        println!("Removed {}", unit_path.display());
    }

    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    println!("StriVo daemon disabled");
    Ok(())
}

fn dirs_home() -> std::path::PathBuf {
    directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
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
            println!("theme = {:?}", cfg.theme);
            if let Some(ref tw) = cfg.twitch {
                println!("twitch.client_id = {:?}", tw.client_id);
                println!("twitch.client_secret = \"****\"");
            } else {
                println!("twitch = <not configured>");
            }
            if let Some(ref yt) = cfg.youtube {
                println!("youtube.client_id = {:?}", yt.client_id);
                println!("youtube.client_secret = \"****\"");
                if let Some(ref cp) = yt.cookies_path {
                    println!("youtube.cookies_path = {:?}", cp.display());
                }
            } else {
                println!("youtube = <not configured>");
            }
            if let Some(ref pa) = cfg.patreon {
                println!("patreon.client_id = {:?}", pa.client_id);
                println!("patreon.client_secret = \"****\"");
                println!("patreon.poll_interval = {}", pa.poll_interval_secs);
            } else {
                println!("patreon = <not configured>");
            }
            if !cfg.auto_record_channels.is_empty() {
                println!();
                println!("auto_record_channels:");
                for entry in &cfg.auto_record_channels {
                    println!("  {} / {} ({})", entry.platform, entry.channel_name, entry.channel_id);
                }
            }
            if !cfg.schedule.is_empty() {
                println!();
                println!("schedule:");
                for entry in &cfg.schedule {
                    println!("  {} | cron: {} | duration: {}", entry.channel, entry.cron, entry.duration);
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
            cfg.twitch = old.twitch;
            cfg.youtube = old.youtube;
            cfg.patreon = old.patreon;
            cfg.auto_record_channels = old.auto_record_channels;
            cfg.schedule = old.schedule;
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
        "twitch.client_secret" => cfg
            .twitch
            .as_ref()
            .map(|t| t.client_secret.clone())
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
        "patreon.client_id" => cfg
            .patreon
            .as_ref()
            .map(|p| p.client_id.clone())
            .ok_or_else(|| anyhow::anyhow!("Patreon not configured")),
        "patreon.client_secret" => cfg
            .patreon
            .as_ref()
            .map(|p| p.client_secret.clone())
            .ok_or_else(|| anyhow::anyhow!("Patreon not configured")),
        "patreon.poll_interval" | "patreon.poll_interval_secs" => cfg
            .patreon
            .as_ref()
            .map(|p| p.poll_interval_secs.to_string())
            .ok_or_else(|| anyhow::anyhow!("Patreon not configured")),
        "theme" => Ok(cfg.theme.clone()),
        _ => Err(anyhow::anyhow!(
            "Unknown key: {key}\n\nValid keys:\n  \
             recording_dir, poll_interval, transcode, filename_template, theme,\n  \
             twitch.client_id, twitch.client_secret,\n  \
             youtube.client_id, youtube.client_secret, youtube.cookies_path,\n  \
             patreon.client_id, patreon.client_secret, patreon.poll_interval"
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
                    client_secret: String::new(),
                });
            }
        }
        "twitch.client_secret" => {
            if let Some(ref mut tw) = cfg.twitch {
                tw.client_secret = value.to_string();
            } else {
                cfg.twitch = Some(config::TwitchConfig {
                    client_id: String::new(),
                    client_secret: value.to_string(),
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
        "patreon.client_id" => {
            if let Some(ref mut pa) = cfg.patreon {
                pa.client_id = value.to_string();
            } else {
                cfg.patreon = Some(config::PatreonConfig {
                    client_id: value.to_string(),
                    client_secret: String::new(),
                    poll_interval_secs: 300,
                });
            }
        }
        "patreon.client_secret" => {
            if let Some(ref mut pa) = cfg.patreon {
                pa.client_secret = value.to_string();
            } else {
                cfg.patreon = Some(config::PatreonConfig {
                    client_id: String::new(),
                    client_secret: value.to_string(),
                    poll_interval_secs: 300,
                });
            }
        }
        "patreon.poll_interval" | "patreon.poll_interval_secs" => {
            let secs: u64 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid integer: {value}"))?;
            if let Some(ref mut pa) = cfg.patreon {
                pa.poll_interval_secs = secs;
            } else {
                cfg.patreon = Some(config::PatreonConfig {
                    client_id: String::new(),
                    client_secret: String::new(),
                    poll_interval_secs: secs,
                });
            }
        }
        "theme" => {
            cfg.theme = value.to_string();
        }
        _ => {
            anyhow::bail!(
                "Unknown key: {key}\n\nValid keys:\n  \
                 recording_dir, poll_interval, transcode, filename_template, theme,\n  \
                 twitch.client_id, twitch.client_secret,\n  \
                 youtube.client_id, youtube.client_secret, youtube.cookies_path,\n  \
                 patreon.client_id, patreon.client_secret, patreon.poll_interval"
            );
        }
    }
    Ok(())
}

async fn handle_log_command(action: &LogAction) -> Result<()> {
    let log_path = config::AppConfig::state_dir().join("strivo.log");

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
        println!("No log file at {}. Start StriVo first.", path.display());
        return Ok(());
    }

    let content = tokio::fs::read_to_string(path).await?;
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(initial_lines);
    for line in &all_lines[start..] {
        println!("{line}");
    }

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
                last_len = 0;
                println!("--- log file truncated ---");
            }
            continue;
        }

        let file = tokio::fs::File::open(path).await?;
        let mut reader = tokio::io::BufReader::new(file);

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

fn handle_search(query: &str, config_path: Option<&std::path::Path>) -> Result<()> {
    let config = config::AppConfig::load(config_path)?;
    let recordings = recording::scan::scan_existing_recordings(&config);

    if recordings.is_empty() {
        println!("No recordings found in {}", config.recording_dir.display());
        return Ok(());
    }

    let query_lower = query.to_lowercase();
    let query_parts: Vec<&str> = query_lower.split_whitespace().collect();

    // Fuzzy match: each query part must either be a substring or fuzzy-match a word
    let mut scored: Vec<(usize, &_)> = recordings
        .iter()
        .filter_map(|rec| {
            let haystack = format!(
                "{} {} {} {}",
                rec.channel_name,
                rec.stream_title.as_deref().unwrap_or(""),
                rec.platform,
                rec.output_path.to_string_lossy(),
            )
            .to_lowercase();

            let mut total_score: usize = 0;
            for part in &query_parts {
                if haystack.contains(part) {
                    // Exact substring match — best score
                    total_score += 0;
                } else if fuzzy_subsequence(part, &haystack) {
                    // Subsequence match (letters appear in order)
                    total_score += 1;
                } else {
                    // Try Levenshtein against individual words
                    let words: Vec<&str> = haystack.split_whitespace().collect();
                    let best = words.iter().map(|w| levenshtein(part, w)).min().unwrap_or(usize::MAX);
                    let threshold = (part.len() / 3).max(1); // allow ~33% edits
                    if best <= threshold {
                        total_score += best;
                    } else {
                        return None; // this query part doesn't match at all
                    }
                }
            }
            Some((total_score, rec))
        })
        .collect();

    // Sort by score (lower = better match)
    scored.sort_by_key(|(score, _)| *score);

    if scored.is_empty() {
        println!("No recordings matching \"{query}\"");
        return Ok(());
    }

    println!(
        "{:<20} {:<10} {:<12} {:<10} {}",
        "Channel", "Platform", "Date", "Size", "Title"
    );
    println!("{}", "─".repeat(80));

    for (_, rec) in &scored {
        let date = rec
            .started_at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string();
        let title = rec
            .stream_title
            .as_deref()
            .unwrap_or("(untitled)");
        let title_display: String = title.chars().take(40).collect();
        println!(
            "{:<20} {:<10} {:<12} {:<10} {}",
            truncate_str(&rec.channel_name, 19),
            rec.platform,
            date,
            rec.format_size(),
            title_display,
        );
    }

    println!("\n{} result(s)", scored.len());
    Ok(())
}

use crate::search::{fuzzy_subsequence, levenshtein};

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

use strivo::check_external_tools;

/// Do one connect+hello+snapshot handshake. Returns `(reader, writer, snapshot)`.
async fn daemon_connect_once(
    socket_path: &std::path::Path,
) -> Result<(
    tokio::io::BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>,
    tokio::io::WriteHalf<tokio::net::UnixStream>,
    ipc::ServerMessage,
)> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = tokio::io::split(stream);
    let mut buf_reader = BufReader::new(reader);

    let hello = ipc::encode_message(&ipc::ClientMessage::Hello)?;
    writer.write_all(hello.as_bytes()).await?;

    let mut line = String::new();
    buf_reader.read_line(&mut line).await?;
    let snapshot: ipc::ServerMessage = serde_json::from_str(line.trim())?;

    Ok((buf_reader, writer, snapshot))
}

/// TUI client mode: connect to running daemon via Unix socket, with a
/// supervised auto-reconnect loop so a daemon restart or crash surfaces as a
/// banner + retry rather than a frozen TUI.
async fn run_client(args: cli::Args) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let config = config::AppConfig::load(args.config.as_deref())?;

    let socket_path = ipc::socket_path();

    // Initial connect — if this fails, fall back to the pre-existing friendly
    // error message because the daemon probably isn't running yet.
    let (mut buf_reader, mut writer, snapshot) = match daemon_connect_once(&socket_path).await {
        Ok(x) => x,
        Err(e) => {
            eprintln!(
                "Failed to connect to daemon at {}: {e}\n\n\
                 Is the daemon running? Start with:\n  \
                 strivo daemon    (foreground)\n  \
                 strivo enable    (systemd service)",
                socket_path.display()
            );
            return Err(e);
        }
    };

    // Create app state from snapshot
    let config_ref = config.clone();
    let mut app_state = app::AppState::new(config);
    if let ipc::ServerMessage::StateSnapshot {
        channels,
        recordings,
        twitch_connected,
        youtube_connected,
        patreon_connected,
        pending_auth,
    } = snapshot
    {
        app_state.channels = channels;
        app_state.recordings = recordings;
        app_state.twitch_connected = twitch_connected;
        app_state.youtube_connected = youtube_connected;
        app_state.patreon_connected = patreon_connected;
        app_state.pending_auth = pending_auth;
        app_state.rebuild_sidebar_order();
    }

    // Channels for daemon communication. `daemon_tx` lives forever in
    // AppState — the supervisor transparently rebinds the underlying
    // socket on reconnect.
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (daemon_tx, mut daemon_rx) = mpsc::unbounded_channel::<ipc::ClientMessage>();

    let (recording_tx, _recording_rx) = mpsc::unbounded_channel();
    app_state.daemon_tx = Some(daemon_tx);

    // Supervisor: pumps reader → event_tx and daemon_rx → writer, reconnects
    // with exponential backoff (1 s, 2 s, 5 s, 10 s, 30 s, 30 s…) on error.
    let event_tx_sup = event_tx.clone();
    let socket_path_sup = socket_path.clone();
    tokio::spawn(async move {
        let mut attempt: u32 = 0;
        loop {
            // Pump until reader or writer breaks.
            let mut line = String::new();
            loop {
                tokio::select! {
                    read = buf_reader.read_line(&mut line) => {
                        match read {
                            Ok(0) => {
                                tracing::info!("Daemon disconnected");
                                break;
                            }
                            Ok(_) => {
                                if let Ok(msg) = serde_json::from_str::<ipc::ServerMessage>(line.trim()) {
                                    match msg {
                                        ipc::ServerMessage::Event(de) => {
                                            let _ = event_tx_sup.send(AppEvent::Daemon(de));
                                        }
                                        ipc::ServerMessage::StateSnapshot {
                                            channels,
                                            twitch_connected,
                                            youtube_connected,
                                            patreon_connected,
                                            ..
                                        } => {
                                            // Re-snapshot after reconnect: push
                                            // state back into the TUI.
                                            let _ = event_tx_sup.send(
                                                AppEvent::channels_updated(channels),
                                            );
                                            if twitch_connected {
                                                let _ = event_tx_sup.send(
                                                    AppEvent::platform_authenticated(
                                                        platform::PlatformKind::Twitch,
                                                    ),
                                                );
                                            }
                                            if youtube_connected {
                                                let _ = event_tx_sup.send(
                                                    AppEvent::platform_authenticated(
                                                        platform::PlatformKind::YouTube,
                                                    ),
                                                );
                                            }
                                            if patreon_connected {
                                                let _ = event_tx_sup.send(
                                                    AppEvent::platform_authenticated(
                                                        platform::PlatformKind::Patreon,
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                                line.clear();
                            }
                            Err(e) => {
                                tracing::warn!("Socket read error: {e}");
                                break;
                            }
                        }
                    }
                    msg = daemon_rx.recv() => {
                        match msg {
                            Some(m) => {
                                if let Ok(encoded) = ipc::encode_message(&m) {
                                    if let Err(e) = writer.write_all(encoded.as_bytes()).await {
                                        tracing::warn!("Socket write error: {e}");
                                        break;
                                    }
                                }
                            }
                            None => {
                                // AppState dropped — TUI is exiting.
                                return;
                            }
                        }
                    }
                }
            }

            // Disconnected. Signal the TUI.
            let _ = event_tx_sup.send(AppEvent::DaemonDisconnected);

            // Reconnect loop with exponential backoff.
            loop {
                attempt = attempt.saturating_add(1);
                let delay_secs: u64 = match attempt {
                    1 => 1,
                    2 => 2,
                    3 => 5,
                    4 => 10,
                    _ => 30,
                };
                tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

                match daemon_connect_once(&socket_path_sup).await {
                    Ok((new_reader, new_writer, _snapshot)) => {
                        buf_reader = new_reader;
                        writer = new_writer;
                        attempt = 0;
                        let _ = event_tx_sup.send(AppEvent::DaemonReconnected);
                        // Next iteration of outer loop resumes pumping on the
                        // new stream. The daemon's initial StateSnapshot was
                        // consumed inside `daemon_connect_once`; subsequent
                        // server-pushed snapshots will flow through normally.
                        break;
                    }
                    Err(e) => {
                        tracing::debug!("Reconnect attempt {attempt} failed: {e}");
                        continue;
                    }
                }
            }
        }
    });

    // Register plugins
    let mut registry = plugin::registry::PluginRegistry::new();
    registry.register(Box::new(plugin::crunchr::CrunchrPlugin::new()));
    registry.register(Box::new(plugin::archiver::ArchiverPlugin::new()));
    registry.init_all(&config_ref)?;

    // Run TUI with the event channel
    tui::run(app_state, registry, event_rx, recording_tx).await?;

    Ok(())
}

/// Standalone TUI mode: runs everything in-process (no daemon).
async fn run_tui(args: cli::Args) -> Result<()> {
    let state_dir = config::AppConfig::state_dir();
    std::fs::create_dir_all(&state_dir)?;

    let log_path = state_dir.join("strivo.log");
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

    tracing::info!("StriVo starting (standalone mode)");

    check_external_tools();

    let config = config::AppConfig::load(args.config.as_deref())?;
    tracing::info!(
        "Config loaded from {}",
        config::AppConfig::config_path().display()
    );

    let cancel = CancellationToken::new();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (recording_tx, recording_rx) = mpsc::unbounded_channel();

    let auth_notify = Arc::new(tokio::sync::Notify::new());

    let mut platforms: Vec<Arc<RwLock<dyn Platform>>> = Vec::new();

    if let Some(ref twitch_config) = config.twitch {
        let mut twitch =
            platform::twitch::TwitchPlatform::new(twitch_config.client_id.clone(), twitch_config.client_secret.clone());
        twitch.set_event_tx(event_tx.clone());
        let twitch = Arc::new(RwLock::new(twitch));
        platforms.push(twitch.clone() as Arc<RwLock<dyn Platform>>);

        let tx = event_tx.clone();
        let notify = auth_notify.clone();
        tokio::spawn(async move {
            let platform = twitch.read().await;
            match platform.authenticate().await {
                Ok(()) => {
                    tracing::info!("Twitch authenticated");
                    let _ = tx.send(AppEvent::platform_authenticated(crate::platform::PlatformKind::Twitch));
                    notify.notify_one();
                }
                Err(e) => {
                    tracing::warn!("Twitch auth failed: {e}");
                    let _ = tx.send(AppEvent::error(format!("Twitch auth: {e}")));
                }
            }
        });
    }

    if let Some(ref yt_config) = config.youtube {
        let mut youtube = platform::youtube::YouTubePlatform::new(
            yt_config.client_id.clone(),
            yt_config.client_secret.clone(),
            yt_config.cookies_path.clone(),
        );
        youtube.set_event_tx(event_tx.clone());
        let youtube = Arc::new(RwLock::new(youtube));
        platforms.push(youtube.clone() as Arc<RwLock<dyn Platform>>);

        let tx = event_tx.clone();
        let notify = auth_notify.clone();
        tokio::spawn(async move {
            let platform = youtube.read().await;
            match platform.authenticate().await {
                Ok(()) => {
                    tracing::info!("YouTube authenticated");
                    let _ = tx.send(AppEvent::platform_authenticated(crate::platform::PlatformKind::YouTube));
                    notify.notify_one();
                }
                Err(e) => {
                    tracing::warn!("YouTube auth failed: {e}");
                    let _ = tx.send(AppEvent::error(format!("YouTube auth: {e}")));
                }
            }
        });
    }

    // Spawn Patreon in standalone mode too
    if let Some(ref patreon_config) = config.patreon {
        let mut patreon_client = crate::platform::patreon::PatreonClient::new(
            patreon_config.client_id.clone(),
            patreon_config.client_secret.clone(),
        );
        patreon_client.set_event_tx(event_tx.clone());

        let tx = event_tx.clone();
        let rec_tx = recording_tx.clone();
        let cfg = config.clone();
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            match patreon_client.authorize().await {
                Ok(()) => {
                    tracing::info!("Patreon authenticated");
                    let _ = tx.send(AppEvent::platform_authenticated(crate::platform::PlatformKind::Patreon));

                    let monitor = crate::monitor::patreon::PatreonMonitor::new(
                        patreon_client,
                        cfg,
                        tx,
                        rec_tx,
                        cancel_clone,
                    );
                    monitor.run().await;
                }
                Err(e) => {
                    tracing::warn!("Patreon auth failed: {e}");
                    let _ = tx.send(AppEvent::error(format!("Patreon auth: {e}")));
                }
            }
        });
    }

    let rec_config = config.clone();
    let rec_tx = event_tx.clone();
    let rec_cancel = cancel.clone();
    tokio::spawn(async move {
        recording::run_manager(rec_config, recording_rx, rec_tx, rec_cancel).await;
    });

    let standalone_poll_notify: Option<Arc<tokio::sync::Notify>> = if !platforms.is_empty() {
        let mut monitor = ChannelMonitor::new(
            platforms.clone(),
            config.clone(),
            event_tx.clone(),
            recording_tx.clone(),
            cancel.clone(),
        );
        monitor.set_auth_notify(auth_notify.clone());
        let poll_notify = monitor.poll_notify();
        tokio::spawn(async move {
            monitor.run().await;
        });
        Some(poll_notify)
    } else {
        None
    };

    // Spawn schedule manager
    if !config.schedule.is_empty() {
        let sched_config = config.clone();
        let sched_rec_tx = recording_tx.clone();
        let sched_event_tx = event_tx.clone();
        let sched_cancel = cancel.clone();
        tokio::spawn(async move {
            recording::schedule::run_schedule_manager(
                sched_config,
                sched_rec_tx,
                sched_event_tx,
                sched_cancel,
            )
            .await;
        });
    }

    let scanned = recording::scan::scan_existing_recordings(&config);

    let mut app_state = app::AppState::new(config.clone());
    app_state.twitch_connected = false;
    app_state.youtube_connected = false;
    app_state.poll_notify_standalone = standalone_poll_notify;

    for job in scanned {
        app_state.recordings.insert(job.id, job);
    }

    // Register plugins
    let mut registry = plugin::registry::PluginRegistry::new();
    registry.register(Box::new(plugin::crunchr::CrunchrPlugin::new()));
    registry.register(Box::new(plugin::archiver::ArchiverPlugin::new()));
    registry.init_all(&config)?;

    tui::run(app_state, registry, event_rx, recording_tx).await?;

    cancel.cancel();

    tracing::info!("StriVo exiting");
    Ok(())
}
