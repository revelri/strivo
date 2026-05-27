// Crate-wide clippy allows — these flag style preferences, not bugs, and
// touching them now would be pure churn. Re-enable per-module when rewriting
// the relevant code.
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::type_complexity)]

mod cli;

use strivo_core::check_external_tools;
use strivo_core::{app, config, daemon, ipc, platform, plugin, recording, tui};

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::cli::{Command, ConfigAction, LogAction, ThemeAction};
use strivo_core::app::AppEvent;
use strivo_core::monitor::ChannelMonitor;
use strivo_core::platform::Platform;
use strivo_core::tui::theme::Theme;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();

    if let Some(ref cmd) = args.command {
        return handle_command(cmd, args.config.as_deref()).await;
    }

    // Initialize theme + motion prefs from config (needed before TUI rendering).
    // M4.6: NO_COLOR forces the monochrome theme regardless of config; the
    // canonical opt-out lets users honor the cross-tool convention without
    // editing their strivo config.
    let no_color = std::env::var("NO_COLOR")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let theme_config = config::AppConfig::load(args.config.as_deref()).ok();
    if no_color {
        Theme::init("monochrome");
    } else if let Some(cfg) = theme_config.as_ref() {
        Theme::init_with_overrides(cfg.theme.name(), cfg.theme.colors(), cfg.theme.ansi());
        // Config flag is an opt-in that layers on top of the env var — either
        // signal enables reduce-motion; neither needs to explicitly disable.
        if cfg.ui.reduce_motion {
            strivo_core::tui::anim::set_reduce_motion(true);
        }
    } else {
        Theme::init("neon");
    }

    // Default: try connecting to daemon, fall back to standalone TUI
    if ipc::is_daemon_running() {
        run_client(args).await
    } else {
        run_tui(args).await
    }
}

async fn handle_command(cmd: &Command, config_path: Option<&std::path::Path>) -> Result<()> {
    match cmd {
        Command::Daemon => {
            // W2 phase 2 — register first-party plugins so they
            // boot inside the daemon process (init_all opens DBs,
            // status_line + properties_section work for the webui).
            let mut host = strivo_core::daemon::DaemonPluginHost::new();
            host.registry
                .register(Box::new(strivo_plugins::crunchr::CrunchrPlugin::new()));
            host.registry
                .register(Box::new(strivo_plugins::archiver::ArchiverPlugin::new()));
            host.registry
                .register(Box::new(strivo_plugins::insights::InsightsPlugin::new()));
            host.registry
                .register(Box::new(strivo_plugins::editor::EditorPlugin::new()));
            host.registry
                .register(Box::new(strivo_plugins::viewguard::ViewguardPlugin::new()));
            daemon::run_with_plugins(host).await
        }
        Command::Enable => handle_enable().await,
        Command::Disable => handle_disable().await,
        Command::Status => handle_status(),
        Command::Config { action } => handle_config_command(action, config_path),
        Command::Log { action } => handle_log_command(action).await,
        Command::Search { query } => handle_search(query, config_path),
        Command::Theme { action } => handle_theme_command(action),
        Command::Doctor => handle_doctor().await,
        Command::Serve { bind, api_key } => handle_serve(bind, api_key.as_deref()).await,
        Command::Chapter { file, every } => handle_chapter(file, *every),
        Command::Import { source } => handle_import(source, config_path),
        Command::Merge { output, sources } => handle_merge(output, sources),
        Command::Thumbnail { file, seek } => handle_thumbnail(file, *seek).await,
        Command::TwitchRewind { channel, sample_secs, out } => {
            handle_twitch_rewind(channel, *sample_secs, out.clone(), config_path).await
        }
        Command::Completions { shell } => handle_completions(*shell),
        Command::Man => handle_man(),
        Command::Pull {
            target,
            format,
            since,
            max,
            force,
            no_transcribe,
        } => {
            handle_pull(
                target,
                format.as_deref(),
                since.as_deref(),
                *max,
                *force,
                *no_transcribe,
                config_path,
            )
            .await
        }
    }
}

fn parse_since(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    let (num_part, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num_part
        .parse()
        .with_context(|| format!("bad --since duration: {s}"))?;
    let dur = match unit {
        "h" => chrono::Duration::hours(n),
        "d" => chrono::Duration::days(n),
        "w" => chrono::Duration::weeks(n),
        _ => anyhow::bail!("unknown --since suffix '{unit}' (use h/d/w or RFC3339)"),
    };
    Ok(chrono::Utc::now() - dur)
}

async fn handle_pull(
    target: &str,
    format_override: Option<&str>,
    since: Option<&str>,
    max: Option<usize>,
    force: bool,
    no_transcribe: bool,
    config_path: Option<&std::path::Path>,
) -> Result<()> {
    use strivo_core::config::{AppConfig, RecordingFormat};
    use strivo_core::platform::{Platform, PlatformKind, VodEntry};
    use strivo_core::recording::catalog::{self, CatalogPullOptions};
    use strivo_core::recording::persist::PersistDb;

    let (platform_str, channel_id) = target.split_once(':').ok_or_else(|| {
        anyhow::anyhow!("target must be `<platform>:<channel_id>`, got `{target}`")
    })?;
    let platform = match platform_str.to_lowercase().as_str() {
        "youtube" | "yt" => PlatformKind::YouTube,
        "twitch" | "tw" => PlatformKind::Twitch,
        "patreon" | "pt" => PlatformKind::Patreon,
        other => anyhow::bail!("unknown platform `{other}` (try youtube/twitch/patreon)"),
    };
    let since = since.map(parse_since).transpose()?;

    let config = AppConfig::load(config_path).context("load config")?;

    // Resolve format: per-channel override → CLI flag → global default → built-ins.
    let cli_override = format_override.map(|f| RecordingFormat {
        format: Some(f.to_string()),
        ..Default::default()
    });
    let chan_override = config
        .auto_record_channels
        .iter()
        .find(|c| c.channel_id == channel_id && c.platform == platform.to_string())
        .and_then(|c| c.format.clone());
    let resolved = RecordingFormat::resolved(
        cli_override.as_ref().or(chan_override.as_ref()),
        &config.recording.format,
    );

    let cookies_path = if matches!(platform, PlatformKind::YouTube) {
        config.youtube.as_ref().and_then(|y| y.cookies_path.clone())
    } else {
        None
    };

    let db_path = AppConfig::data_dir().join("jobs.db");
    let db = PersistDb::open(&db_path).context("open jobs.db")?;

    println!("Enumerating {platform} catalog for {channel_id}…");
    let vods: Vec<VodEntry> = match platform {
        PlatformKind::YouTube => {
            let yt_cfg = config
                .youtube
                .clone()
                .context("youtube section missing in config")?;
            let yt = strivo_core::platform::youtube::YouTubePlatform::new(
                yt_cfg.client_id,
                yt_cfg.client_secret,
                yt_cfg.cookies_path.clone(),
            );
            yt.load_stored_tokens().await.context("youtube auth")?;
            yt.fetch_channel_vods(channel_id, since, max).await?
        }
        PlatformKind::Twitch => {
            let tw_cfg = config
                .twitch
                .clone()
                .context("twitch section missing in config")?;
            let tw = strivo_core::platform::twitch::TwitchPlatform::new(
                tw_cfg.client_id,
                tw_cfg.client_secret,
            );
            tw.load_stored_tokens().await.context("twitch auth")?;
            tw.fetch_channel_vods(channel_id, since, max).await?
        }
        PlatformKind::Patreon => {
            let pt_cfg = config
                .patreon
                .clone()
                .context("patreon section missing in config")?;
            let pt = strivo_core::platform::patreon::PatreonClient::new(
                pt_cfg.client_id,
                pt_cfg.client_secret,
            );
            pt.load_stored_tokens().await.context("patreon auth")?;
            pt.fetch_channel_vods(channel_id, since, max).await?
        }
    };

    if vods.is_empty() {
        println!("No matching VODs found.");
        return Ok(());
    }
    println!("Discovered {} VOD(s).", vods.len());

    let opts = CatalogPullOptions {
        root: config.recording_dir.clone(),
        channel_name: channel_id.to_string(),
        format: resolved,
        cookies_path,
        force,
        crunchr_auto: !no_transcribe && config.crunchr.enabled,
    };

    let report = catalog::run_pull(&db, vods, &opts, None, None).await?;
    println!(
        "Done. discovered={} skipped={} downloaded={} failed={}",
        report.discovered,
        report.skipped_existing,
        report.downloaded,
        report.failed.len()
    );
    for (id, err) in &report.failed {
        eprintln!("  failed: {id} — {err}");
    }
    Ok(())
}

fn handle_completions(shell: clap_complete::Shell) -> Result<()> {
    let mut cmd = cli::Args::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
    Ok(())
}

fn handle_man() -> Result<()> {
    let cmd = cli::Args::command();
    let man = clap_mangen::Man::new(cmd);
    man.render(&mut std::io::stdout())?;
    Ok(())
}

fn handle_theme_command(action: &ThemeAction) -> Result<()> {
    use strivo_core::tui::theme;
    match action {
        ThemeAction::List => {
            let current = config::AppConfig::load(None)
                .map(|c| c.theme.name().to_string())
                .unwrap_or_else(|_| "neon".to_string());
            let names = theme::available_themes();
            let builtins: std::collections::HashSet<String> = theme::builtin_themes()
                .into_iter()
                .map(|t| t.name)
                .collect();
            println!("Themes ({} total)", names.len());
            for n in &names {
                let marker = if *n == current { "*" } else { " " };
                let source = if builtins.contains(n) {
                    "built-in"
                } else {
                    "user"
                };
                println!("  {marker} {n}  [{source}]");
            }
            Ok(())
        }
        ThemeAction::Import { path, name } => {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("read {}", path.display()))?;
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported");
            let theme_name = name.clone().unwrap_or_else(|| stem.to_string());
            let theme = theme::kitty_import::parse(&theme_name, &contents)
                .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;

            let dest_dir = config::AppConfig::config_dir().join("themes");
            std::fs::create_dir_all(&dest_dir)
                .with_context(|| format!("create {}", dest_dir.display()))?;
            let dest = dest_dir.join(format!("{theme_name}.toml"));
            let serialized = toml::to_string_pretty(&theme).context("serialize imported theme")?;
            std::fs::write(&dest, serialized)
                .with_context(|| format!("write {}", dest.display()))?;
            println!("Imported '{theme_name}' → {}", dest.display());
            println!("Activate with: strivo config set theme {theme_name}");
            Ok(())
        }
    }
}

fn handle_import(source: &cli::ImportSource, config_path: Option<&std::path::Path>) -> Result<()> {
    use strivo_core::config::import::{parse_obs_export, parse_streamlink_lines, Candidate};

    let (candidates, apply, source_path) = match source {
        cli::ImportSource::Obs { file, apply } => (parse_obs_export(file)?, *apply, file.clone()),
        cli::ImportSource::Streamlink { file, apply } => {
            (parse_streamlink_lines(file)?, *apply, file.clone())
        }
    };

    if candidates.is_empty() {
        println!("No channels discovered in {}", source_path.display());
        return Ok(());
    }

    println!("Discovered {} channel(s):", candidates.len());
    for c in &candidates {
        println!("  + {}:{}  ({})", c.platform, c.channel_id, c.channel_name);
    }

    if !apply {
        println!();
        println!("Dry-run. Pass --apply to write into config.toml.");
        return Ok(());
    }

    let mut cfg = config::AppConfig::load(config_path).context("load config")?;
    let mut added = 0usize;
    let mut skipped = 0usize;
    for c in candidates {
        let exists = cfg
            .auto_record_channels
            .iter()
            .any(|a| a.platform == c.platform && a.channel_id == c.channel_id);
        if exists {
            skipped += 1;
            continue;
        }
        cfg.auto_record_channels
            .push(Candidate::into_auto_record(c));
        added += 1;
    }
    cfg.save(config_path).context("save config")?;
    println!("Applied: {added} added, {skipped} already present.");
    Ok(())
}

fn handle_merge(output: &std::path::Path, sources: &[std::path::PathBuf]) -> Result<()> {
    use strivo_core::recording::segments::merge_segments;
    if sources.is_empty() {
        anyhow::bail!("provide at least one source file");
    }
    println!(
        "Merging {} segment(s) → {}",
        sources.len(),
        output.display()
    );
    merge_segments(sources, output)?;
    println!("ok");
    Ok(())
}

/// Debug subcommand: resolve the Twitch rewind master playlist for a live
/// channel and optionally smoke-test it by pulling a few seconds with
/// ffmpeg. Verifies the GQL → Usher → segment chain end-to-end.
async fn handle_twitch_rewind(
    channel: &str,
    sample_secs: Option<u32>,
    out: Option<std::path::PathBuf>,
    config_path: Option<&std::path::Path>,
) -> Result<()> {
    use std::sync::Arc;
    use strivo_core::stream::twitch_rewind::RewindResolver;
    use tokio::sync::RwLock;

    let config = config::AppConfig::load(config_path).context("load config")?;
    let tw_cfg = config
        .twitch
        .clone()
        .context("twitch section missing in config")?;
    let tw = strivo_core::platform::twitch::TwitchPlatform::new(
        tw_cfg.client_id.clone(),
        tw_cfg.client_secret.clone(),
    );
    tw.load_stored_tokens()
        .await
        .context("load twitch tokens (run `strivo` once to authenticate)")?;

    let (channel_id, _display_name) = tw
        .lookup_channel_id_by_login(channel)
        .await
        .context("lookup channel id")?;
    println!("channel_id = {channel_id}");

    let token = strivo_core::config::credentials::get_secret("twitch_access_token")
        .ok()
        .flatten();
    let tw_arc = Arc::new(RwLock::new(tw));
    let resolver = RewindResolver::new(tw_arc, token);
    let stream = match resolver.resolve(&channel_id).await {
        Ok(s) => s,
        Err(e) => anyhow::bail!("rewind resolve failed: {e}"),
    };
    println!("video_id = {}", stream.video_id);
    if let Some(t) = stream.broadcast_started_at {
        println!("broadcast_started_at = {t}");
    }
    println!("master_url = {}", stream.master_url);

    if let Some(secs) = sample_secs {
        let out_path = out.unwrap_or_else(|| {
            std::path::PathBuf::from(format!("./rewind-sample-{channel}.mkv"))
        });
        println!("\nffmpeg smoke test: pulling first {secs}s into {}...", out_path.display());
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "warning", "-y", "-t"])
            .arg(secs.to_string())
            .args(["-i"])
            .arg(&stream.master_url)
            .args(["-c", "copy"])
            .arg(&out_path)
            .status()
            .await
            .context("spawn ffmpeg")?;
        if !status.success() {
            anyhow::bail!("ffmpeg exited with {status}");
        }
        let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
        println!("ok — wrote {} bytes to {}", size, out_path.display());
    }
    Ok(())
}

async fn handle_thumbnail(file: &std::path::Path, seek: f64) -> Result<()> {
    use strivo_core::recording::thumbnail;
    if !file.exists() {
        anyhow::bail!("file does not exist: {}", file.display());
    }
    if let Some(cached) = thumbnail::cached(file) {
        println!("cached: {}", cached.display());
        return Ok(());
    }
    let path = thumbnail::extract(file, seek)
        .await
        .context("extract thumbnail")?;
    println!("wrote: {}", path.display());
    Ok(())
}

fn handle_chapter(file: &std::path::Path, every: u64) -> Result<()> {
    use strivo_core::media::probe_file;
    use strivo_core::recording::chapters::{embed_chapters, every_n_minutes};

    if !file.exists() {
        anyhow::bail!("file does not exist: {}", file.display());
    }
    // Probe the duration so the chapter set reaches the end of the file.
    let info = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(probe_file(file))
        .context("ffprobe the recording duration")?;
    let duration = info.duration_secs;
    if duration <= 0.0 {
        anyhow::bail!("ffprobe reported zero duration; cannot chapter");
    }
    let chapters = every_n_minutes(duration, every);
    if chapters.is_empty() {
        anyhow::bail!("no chapters generated (file shorter than interval?)");
    }
    println!(
        "Embedding {} chapter(s) every {} min into {}",
        chapters.len(),
        every,
        file.display()
    );
    embed_chapters(file, &chapters)?;
    println!("ok");
    Ok(())
}

async fn handle_serve(bind: &str, api_key: Option<&str>) -> Result<()> {
    let addr: std::net::SocketAddr = bind
        .parse()
        .with_context(|| format!("invalid --bind {bind}"))?;

    // Key precedence: explicit --api-key > config.toml `[web] api_key`
    // > freshly generated + saved.
    let mut cfg = config::AppConfig::load(None).context("load config")?;
    let api_key = if let Some(k) = api_key {
        strivo_web::auth::ApiKey(k.to_string())
    } else if let Some(k) = cfg.web.api_key.clone() {
        strivo_web::auth::ApiKey(k)
    } else {
        let generated = strivo_web::auth::ApiKey::generate();
        cfg.web.api_key = Some(generated.as_str().to_string());
        if let Err(e) = cfg.save(None) {
            tracing::warn!("could not persist [web] api_key to config.toml: {e}");
        }
        generated
    };

    println!(
        "strivo-web on http://{} (X-Api-Key: {})",
        addr,
        api_key.as_str()
    );
    strivo_web::serve(strivo_web::ServeConfig {
        bind: addr,
        api_key,
    })
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))
}

async fn handle_doctor() -> Result<()> {
    // Tool presence first, then platform credentials so the user sees
    // both gates in one shot. We're already inside the #[tokio::main]
    // runtime, so the credential probes are awaited directly — spinning
    // a nested current-thread runtime here panics with "Cannot start a
    // runtime from within a runtime".
    let creds_summary = probe_platform_credentials().await;

    let tools: &[(&str, &str)] = &[
        ("ffmpeg", "recording (required)"),
        ("mpv", "playback (required)"),
        ("streamlink", "Twitch stream resolution (required)"),
        ("yt-dlp", "YouTube/Patreon resolution (required)"),
        ("whisper", "transcription (optional, Crunchr plugin)"),
    ];
    let mut missing_required = 0;
    println!("StriVo external tool check");
    println!("{}", "-".repeat(60));
    for (bin, purpose) in tools {
        match which::which(bin) {
            Ok(path) => println!("  ok      {:<12} {}  [{}]", bin, purpose, path.display()),
            Err(_) => {
                println!("  MISSING {:<12} {}", bin, purpose);
                if purpose.contains("required") {
                    missing_required += 1;
                }
            }
        }
    }
    println!();
    if missing_required > 0 {
        println!(
            "{} required tool(s) missing. Install via: pacman -S ffmpeg mpv streamlink yt-dlp",
            missing_required
        );
        std::process::exit(1);
    } else {
        println!("All required tools present.");
    }

    println!();
    println!("Platform credentials");
    println!("{}", "-".repeat(60));
    print!("{creds_summary}");
    Ok(())
}

/// Test each configured platform's stored credentials by attempting a
/// lightweight authenticated call. The wizard-credential-validation
/// item — surfaces stale tokens immediately rather than waiting for the
/// next monitor poll to fail.
async fn probe_platform_credentials() -> String {
    use strivo_core::config::AppConfig;
    use strivo_core::platform::Platform;

    let cfg = match AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return format!("  could not load config: {e}\n"),
    };
    let mut out = String::new();

    if let Some(ref tw) = cfg.twitch {
        let plat = strivo_core::platform::twitch::TwitchPlatform::new(
            tw.client_id.clone(),
            tw.client_secret.clone(),
        );
        match plat.load_stored_tokens().await {
            Ok(true) => match plat.fetch_followed_channels().await {
                Ok(channels) => {
                    out.push_str(&format!(
                        "  ok      twitch       {} followed channel(s)\n",
                        channels.len()
                    ));
                }
                Err(e) => out.push_str(&format!(
                    "  STALE   twitch       call failed: {e}\n  hint: re-run the wizard or 'strivo config reset' the twitch keys\n"
                )),
            },
            Ok(false) => out.push_str("  none    twitch       no stored token (run the wizard)\n"),
            Err(e) => out.push_str(&format!("  ERROR   twitch       {e}\n")),
        }
    } else {
        out.push_str("  skip    twitch       not configured\n");
    }

    if let Some(ref yt) = cfg.youtube {
        let plat = strivo_core::platform::youtube::YouTubePlatform::new(
            yt.client_id.clone(),
            yt.client_secret.clone(),
            yt.cookies_path.clone(),
        );
        match plat.load_stored_tokens().await {
            Ok(true) => match plat.fetch_followed_channels().await {
                Ok(channels) => out.push_str(&format!(
                    "  ok      youtube      {} subscription(s)\n",
                    channels.len()
                )),
                Err(e) => out.push_str(&format!(
                    "  STALE   youtube      call failed: {e}\n  hint: re-auth or refresh the cookies file\n"
                )),
            },
            Ok(false) => out.push_str("  none    youtube      no stored token (run the wizard)\n"),
            Err(e) => out.push_str(&format!("  ERROR   youtube      {e}\n")),
        }
    } else {
        out.push_str("  skip    youtube      not configured\n");
    }

    if let Some(ref pt) = cfg.patreon {
        let plat = strivo_core::platform::patreon::PatreonClient::new(
            pt.client_id.clone(),
            pt.client_secret.clone(),
        );
        match plat.load_stored_tokens().await {
            Ok(true) => match plat.fetch_pledged_creators().await {
                Ok(creators) => out.push_str(&format!(
                    "  ok      patreon      {} pledged creator(s)\n",
                    creators.len()
                )),
                Err(e) => out.push_str(&format!("  STALE   patreon      call failed: {e}\n")),
            },
            Ok(false) => out.push_str("  none    patreon      no stored token\n"),
            Err(e) => out.push_str(&format!("  ERROR   patreon      {e}\n")),
        }
    } else {
        out.push_str("  skip    patreon      not configured\n");
    }

    out
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

fn handle_config_command(
    action: &ConfigAction,
    config_path: Option<&std::path::Path>,
) -> Result<()> {
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
            println!(
                "recording.filename_template = {:?}",
                cfg.recording.filename_template
            );
            println!("theme = {:?}", cfg.theme.name());
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
                    println!(
                        "  {} / {} ({})",
                        entry.platform, entry.channel_name, entry.channel_id
                    );
                }
            }
            if !cfg.schedule.is_empty() {
                println!();
                println!("schedule:");
                for entry in &cfg.schedule {
                    println!(
                        "  {} | cron: {} | duration: {}",
                        entry.channel, entry.cron, entry.duration
                    );
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
        "theme" => Ok(cfg.theme.name().to_string()),
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
                    websub_callback_url: None,
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
                    websub_callback_url: None,
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
                    websub_callback_url: None,
                });
            }
        }
        "youtube.websub_callback_url" => {
            if let Some(ref mut yt) = cfg.youtube {
                yt.websub_callback_url = Some(value.to_string());
            } else {
                cfg.youtube = Some(config::YouTubeConfig {
                    client_id: String::new(),
                    client_secret: String::new(),
                    cookies_path: None,
                    websub_callback_url: Some(value.to_string()),
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
                    cookies_path: None,
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
                    cookies_path: None,
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
                    cookies_path: None,
                });
            }
        }
        "patreon.cookies_path" => {
            if let Some(ref mut pa) = cfg.patreon {
                pa.cookies_path = Some(std::path::PathBuf::from(value));
            } else {
                cfg.patreon = Some(config::PatreonConfig {
                    client_id: String::new(),
                    client_secret: String::new(),
                    poll_interval_secs: 300,
                    cookies_path: Some(std::path::PathBuf::from(value)),
                });
            }
        }
        "theme" => {
            cfg.theme.set_name(value.to_string());
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
                    let best = words
                        .iter()
                        .map(|w| levenshtein(part, w))
                        .min()
                        .unwrap_or(usize::MAX);
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
        "{:<20} {:<10} {:<12} {:<10} Title",
        "Channel", "Platform", "Date", "Size"
    );
    println!("{}", "─".repeat(80));

    for (_, rec) in &scored {
        let date = rec
            .started_at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string();
        let title = rec.stream_title.as_deref().unwrap_or("(untitled)");
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

use strivo_core::search::{fuzzy_subsequence, levenshtein};

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

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
        ..
    } = snapshot
    {
        app_state.channels = channels;
        app_state.recordings = recordings;
        // Apply persisted watch history to the initial snapshot.
        for (id, job) in app_state.recordings.iter_mut() {
            if app_state.state.watched.contains(id) {
                job.watched = true;
            }
        }
        app_state.twitch_connected = twitch_connected;
        app_state.youtube_connected = youtube_connected;
        app_state.patreon_connected = patreon_connected;
        app_state.pending_auth = pending_auth;
        app_state.rebuild_sidebar_order();
        app_state.reconcile_selected_recording();
    }

    // Channels for daemon communication. `daemon_tx` lives forever in
    // AppState — the supervisor transparently rebinds the underlying
    // socket on reconnect.
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (daemon_tx, mut daemon_rx) = mpsc::unbounded_channel::<ipc::ClientMessage>();

    // Tracing → event-ring bridge: the registered Layer forwards events
    // from `tracing::{info,warn,error}!` macros (in strivo* targets) onto
    // this channel as AppEvent::LogBridge. AppState.event_ring picks them
    // up alongside daemon events.
    strivo_core::tui::log_bridge::install_sender(event_tx.clone());

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

    // Register plugins (first-party in-tree + dynamically-loaded
    // cdylibs declared in user manifests).
    let mut registry = plugin::registry::PluginRegistry::new();
    registry.register(Box::new(strivo_plugins::crunchr::CrunchrPlugin::new()));
    registry.register(Box::new(strivo_plugins::archiver::ArchiverPlugin::new()));
    registry.register(Box::new(strivo_plugins::insights::InsightsPlugin::new()));
    registry.register(Box::new(strivo_plugins::editor::EditorPlugin::new()));
    registry.register(Box::new(strivo_plugins::viewguard::ViewguardPlugin::new()));
    let manifests = strivo_core::plugin::scan_user_plugins(&strivo_core::plugin::user_plugin_dir());
    let n = registry.load_dylibs_from_manifests(&manifests);
    if n > 0 {
        tracing::info!("loaded {n} dynamic plugin(s)");
    }
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

    // File-tail layer + in-memory bridge to AppState.event_ring.
    use tracing_subscriber::prelude::*;
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&args.log_level));
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(log_file)
        .with_ansi(false);
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(strivo_core::tui::log_bridge::LogBridgeLayer)
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
    let mut twitch_handle: Option<Arc<RwLock<platform::twitch::TwitchPlatform>>> = None;

    if let Some(ref twitch_config) = config.twitch {
        let mut twitch = platform::twitch::TwitchPlatform::new(
            twitch_config.client_id.clone(),
            twitch_config.client_secret.clone(),
        );
        twitch.set_event_tx(event_tx.clone());
        let twitch = Arc::new(RwLock::new(twitch));
        platforms.push(twitch.clone() as Arc<RwLock<dyn Platform>>);
        twitch_handle = Some(twitch.clone());

        let tx = event_tx.clone();
        let notify = auth_notify.clone();
        tokio::spawn(async move {
            let platform = twitch.read().await;
            match platform.authenticate().await {
                Ok(()) => {
                    tracing::info!("Twitch authenticated");
                    let _ = tx.send(AppEvent::platform_authenticated(
                        strivo_core::platform::PlatformKind::Twitch,
                    ));
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
                    let _ = tx.send(AppEvent::platform_authenticated(
                        strivo_core::platform::PlatformKind::YouTube,
                    ));
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
        let mut patreon_client = strivo_core::platform::patreon::PatreonClient::new(
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
                    let _ = tx.send(AppEvent::platform_authenticated(
                        strivo_core::platform::PlatformKind::Patreon,
                    ));

                    let monitor = strivo_core::monitor::patreon::PatreonMonitor::new(
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
    let rec_twitch = twitch_handle.clone();
    tokio::spawn(async move {
        recording::run_manager(rec_config, rec_twitch, recording_rx, rec_tx, rec_cancel).await;
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
    registry.register(Box::new(strivo_plugins::crunchr::CrunchrPlugin::new()));
    registry.register(Box::new(strivo_plugins::archiver::ArchiverPlugin::new()));
    registry.register(Box::new(strivo_plugins::insights::InsightsPlugin::new()));
    registry.register(Box::new(strivo_plugins::editor::EditorPlugin::new()));
    registry.register(Box::new(strivo_plugins::viewguard::ViewguardPlugin::new()));
    registry.init_all(&config)?;

    tui::run(app_state, registry, event_rx, recording_tx).await?;

    cancel.cancel();

    tracing::info!("StriVo exiting");
    Ok(())
}
