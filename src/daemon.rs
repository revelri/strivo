use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app::{AppEvent, DaemonEvent};
use crate::config::AppConfig;
use crate::ipc::{self, ClientMessage, ServerMessage};
use crate::monitor::ChannelMonitor;
use crate::platform::{ChannelEntry, Platform, PlatformKind};
use crate::recording::job::RecordingJob;
use crate::recording::RecordingCommand;

/// Cap on retained terminal (Finished/Failed) recordings. Active jobs are
/// never evicted; this only bounds the completed-history tail so the map
/// can't grow without limit over a long-running daemon (roadmap item 8).
const MAX_TERMINAL_RECORDINGS: usize = 200;

/// Cap on concurrently-handled client connections (TUI + webui share the
/// daemon over the Unix socket). Excess connections are dropped rather than
/// queued so the accept loop can't be starved (roadmap item 9).
const MAX_CLIENT_TASKS: usize = 64;

/// Daemon state — maintained from internal events.
struct DaemonState {
    channels: Vec<ChannelEntry>,
    recordings: HashMap<Uuid, RecordingJob>,
    twitch_connected: bool,
    youtube_connected: bool,
    patreon_connected: bool,
    pending_auth: Option<(PlatformKind, String, String)>,
    auth_queue: std::collections::VecDeque<(PlatformKind, String, String)>,
    // Latest Patreon snapshot, cached so a client connecting between polls
    // sees Patreon immediately (not after up to a full poll interval).
    patreon_creators: Vec<ChannelEntry>,
    patreon_posts: Vec<crate::platform::patreon::PatreonPost>,
}

impl DaemonState {
    fn snapshot(&self) -> ServerMessage {
        ServerMessage::StateSnapshot {
            channels: self.channels.clone(),
            recordings: self.recordings.clone(),
            twitch_connected: self.twitch_connected,
            youtube_connected: self.youtube_connected,
            patreon_connected: self.patreon_connected,
            pending_auth: self.pending_auth.clone(),
            patreon_creators: self.patreon_creators.clone(),
            patreon_posts: self.patreon_posts.clone(),
        }
    }

    /// Drop the oldest terminal recordings beyond [`MAX_TERMINAL_RECORDINGS`].
    /// Active jobs (ResolvingUrl/Recording/Stopping) are always kept.
    fn evict_old_terminal(&mut self) {
        use crate::recording::job::RecordingState;
        let mut terminal: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> = self
            .recordings
            .iter()
            .filter(|(_, j)| matches!(j.state, RecordingState::Finished | RecordingState::Failed))
            .map(|(id, j)| (*id, j.started_at))
            .collect();
        if terminal.len() <= MAX_TERMINAL_RECORDINGS {
            return;
        }
        // Oldest first; remove everything beyond the cap.
        terminal.sort_by_key(|(_, started)| *started);
        let remove = terminal.len() - MAX_TERMINAL_RECORDINGS;
        for (id, _) in terminal.into_iter().take(remove) {
            self.recordings.remove(&id);
        }
    }

    fn apply(&mut self, event: &DaemonEvent) {
        match event {
            DaemonEvent::ChannelsUpdated(channels) => {
                self.channels = channels.clone();
            }
            DaemonEvent::PatreonState { creators, posts } => {
                self.patreon_creators = creators.clone();
                self.patreon_posts = posts.clone();
            }
            DaemonEvent::RecordingStarted { job } => {
                self.recordings.insert(job.id, job.clone());
            }
            DaemonEvent::RecordingProgress {
                job_id,
                bytes_written,
                duration_secs,
            } => {
                if let Some(job) = self.recordings.get_mut(job_id) {
                    job.bytes_written = *bytes_written;
                    job.duration_secs = *duration_secs;
                    job.state = crate::recording::job::RecordingState::Recording;
                }
            }
            DaemonEvent::RecordingFinished {
                job_id,
                final_state,
                error,
            } => {
                if let Some(job) = self.recordings.get_mut(job_id) {
                    job.state = *final_state;
                    job.error = error.clone();
                }
                self.evict_old_terminal();
            }
            DaemonEvent::DeviceCodeRequired {
                kind,
                verification_uri,
                user_code,
            } => {
                let entry = (*kind, verification_uri.clone(), user_code.clone());
                if matches!(&self.pending_auth, Some((p, _, _)) if *p == entry.0) {
                    self.pending_auth = Some(entry);
                } else {
                    self.auth_queue.retain(|(p, _, _)| *p != entry.0);
                    if self.pending_auth.is_none() {
                        self.pending_auth = Some(entry);
                    } else {
                        self.auth_queue.push_back(entry);
                    }
                }
            }
            DaemonEvent::PlatformAuthenticated { kind } => {
                match kind {
                    PlatformKind::Twitch => self.twitch_connected = true,
                    PlatformKind::YouTube => self.youtube_connected = true,
                    PlatformKind::Patreon => self.patreon_connected = true,
                }
                if matches!(&self.pending_auth, Some((pending, _, _)) if pending == kind) {
                    self.pending_auth = self.auth_queue.pop_front();
                }
                self.auth_queue.retain(|(p, _, _)| p != kind);
            }
            _ => {}
        }
    }
}

/// Daemon-side plugin host. (W2 phase 2.)
///
/// The daemon used to ignore plugins entirely — they were a TUI / bin
/// concern. With the webui's PluginRpc surface, plugins need to be
/// alive inside the daemon process too so their DB hooks, status_line
/// contributions, and (eventually) verb dispatchers have somewhere to
/// run.
///
/// Verb dispatch over IPC is a phase-3 follow-up — it requires a
/// minimal "DaemonAppState" wrapper for plugins to read recordings
/// from, since the full AppState is TUI-scoped. Today the daemon
/// loads + initializes plugins, runs their event hooks, and logs any
/// PluginRpc requests. The wire contract is stable; only the
/// dispatcher body changes.
pub struct DaemonPluginHost {
    pub registry: crate::plugin::registry::PluginRegistry,
}

impl DaemonPluginHost {
    pub fn new() -> Self {
        Self {
            registry: crate::plugin::registry::PluginRegistry::new(),
        }
    }
}

impl Default for DaemonPluginHost {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn run() -> Result<()> {
    run_with_plugins(DaemonPluginHost::new()).await
}

pub async fn run_with_plugins(host: DaemonPluginHost) -> Result<()> {
    // Initialize logging
    let state_dir = AppConfig::state_dir();
    std::fs::create_dir_all(&state_dir)?;

    // Rolling, capped log files (roadmap item 15): daily rotation, keep the
    // last 7 days, so logs never grow unbounded and users never SSH for them
    // (the webui tails the newest file). Files are `strivo.<date>.log`.
    let appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("strivo")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&state_dir)
        .context("build rolling log appender")?;
    let (nb_writer, log_guard) = tracing_appender::non_blocking(appender);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(nb_writer)
        .with_ansi(false)
        .init();
    // Keep the non-blocking writer's flush guard alive for the daemon's
    // lifetime; dropping it would stop log flushing.
    let _log_guard = log_guard;

    tracing::info!("StriVo daemon starting");

    // Write PID file
    let pid_path = ipc::pid_path();
    std::fs::write(&pid_path, std::process::id().to_string())?;

    // Validate external tools
    crate::check_external_tools();

    // Load config
    let config = AppConfig::load(None)?;
    tracing::info!("Config loaded");
    for w in config.config_warnings() {
        tracing::warn!("config: {w}");
    }

    // W2 phase 2 — init plugins inside the daemon process. Plugins
    // are registered by the caller (strivo-bin's Command::Daemon
    // arm) via DaemonPluginHost.registry; init_all opens their
    // DBs, sets up tandem state, etc. Verb dispatch is still
    // logging-only until the AppState wrapper lands (W2-phase-3).
    let mut host = host;
    if !host.registry.is_empty() {
        match host.registry.init_all(&config) {
            Ok(()) => {
                tracing::info!(
                    plugin_count = host.registry.len(),
                    "daemon: plugin host initialized"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "daemon: plugin host init failed; continuing without plugin features"
                );
            }
        }
    }
    // W2-phase-3: share the registry with per-connection handlers so
    // PluginRpc over IPC can actually dispatch on_verb (it's idle after
    // init_all otherwise). tokio Mutex — dispatch is sync and brief.
    let registry = std::sync::Arc::new(tokio::sync::Mutex::new(host.registry));

    // Open the persistence db (jobs / catalog / crunchr_queue) and recover any
    // jobs that were marked running when the daemon last died. Recovery is
    // intentionally minimal: we mark orphans as 'interrupted' so the audit log
    // is honest. Catalog-pull resumption is automatic — the catalog dedupe
    // index in §5 already skips already-recorded VODs on the next pull.
    let persist_db =
        match crate::recording::persist::PersistDb::open(&AppConfig::data_dir().join("jobs.db")) {
            Ok(db) => {
                match db.recover_orphaned_running().await {
                    Ok(n) if n > 0 => {
                        tracing::info!("daemon: marked {n} orphan job(s) as interrupted")
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("daemon: persist recover failed: {e}"),
                }
                Some(Arc::new(db))
            }
            Err(e) => {
                tracing::warn!("daemon: failed to open jobs.db: {e} — durability disabled");
                None
            }
        };

    let cancel = CancellationToken::new();

    // Internal event channel
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    // Recording command channel
    let (recording_tx, recording_rx) = mpsc::unbounded_channel();

    // Broadcast channel for client fan-out
    let (broadcast_tx, _) = broadcast::channel::<DaemonEvent>(256);

    // Shared auth notify
    let auth_notify = Arc::new(tokio::sync::Notify::new());

    // Initialize platforms
    let mut platforms: Vec<Arc<RwLock<dyn Platform>>> = Vec::new();
    let mut twitch_handle: Option<Arc<RwLock<crate::platform::twitch::TwitchPlatform>>> = None;

    if let Some(ref twitch_config) = config.twitch {
        let mut twitch = crate::platform::twitch::TwitchPlatform::new(
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
                    let _ = tx.send(AppEvent::platform_authenticated(PlatformKind::Twitch));
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
        let mut youtube = crate::platform::youtube::YouTubePlatform::new(
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
                    let _ = tx.send(AppEvent::platform_authenticated(PlatformKind::YouTube));
                    notify.notify_one();
                }
                Err(e) => {
                    tracing::warn!("YouTube auth failed: {e}");
                    let _ = tx.send(AppEvent::error(format!("YouTube auth: {e}")));
                }
            }
        });
    }

    // Spawn Patreon auth + monitor
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
                    let _ = tx.send(AppEvent::platform_authenticated(PlatformKind::Patreon));

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

    // Spawn recording manager
    let rec_config = config.clone();
    let rec_tx = event_tx.clone();
    let rec_cancel = cancel.clone();
    let rec_twitch = twitch_handle.clone();
    tokio::spawn(async move {
        crate::recording::run_manager(rec_config, rec_twitch, recording_rx, rec_tx, rec_cancel).await;
    });

    // Spawn the per-channel bulk back-catalog download manager (task #71).
    let bulk_tx = crate::recording::bulk::spawn(config.clone(), event_tx.clone());

    // Spawn channel monitor
    let mut interval_ctl: Option<(
        std::sync::Arc<std::sync::atomic::AtomicU64>,
        std::sync::Arc<tokio::sync::Notify>,
    )> = None;
    let poll_notify = if !platforms.is_empty() {
        let mut monitor = ChannelMonitor::new(
            platforms.clone(),
            config.clone(),
            event_tx.clone(),
            recording_tx.clone(),
            cancel.clone(),
        );
        monitor.set_auth_notify(auth_notify.clone());
        if let Some(ref db) = persist_db {
            monitor.set_persist(db.clone());
        }
        let poll_notify = monitor.poll_notify();
        interval_ctl = Some(monitor.interval_controls());
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
            crate::recording::schedule::run_schedule_manager(
                sched_config,
                sched_rec_tx,
                sched_event_tx,
                sched_cancel,
            )
            .await;
        });
    }

    // Scan existing recordings
    let scanned = crate::recording::scan::scan_existing_recordings(&config);

    // Initialize daemon state
    let mut state = DaemonState {
        channels: Vec::new(),
        recordings: HashMap::new(),
        twitch_connected: false,
        youtube_connected: false,
        patreon_connected: false,
        pending_auth: None,
        auth_queue: std::collections::VecDeque::new(),
        patreon_creators: Vec::new(),
        patreon_posts: Vec::new(),
    };
    for job in scanned {
        state.recordings.insert(job.id, job);
    }

    // Replay recordings from the journal so the TUI sees jobs that
    // started before a crash (with their original IDs, channel links,
    // and last-known progress). Disk-scan above is the backstop for
    // jobs that pre-date the journal; journal wins on conflict because
    // it preserves the original Uuid and metadata.
    if let Some(ref db) = persist_db {
        match db.load_recording_jobs().await {
            Ok(jobs) => {
                let n = jobs.len();
                for job in jobs {
                    state.recordings.insert(job.id, job);
                }
                if n > 0 {
                    tracing::info!("daemon: replayed {n} recording(s) from journal");
                }
            }
            Err(e) => tracing::warn!("daemon: failed to load recording journal: {e}"),
        }
    }

    // Set up Unix socket
    let socket_path = ipc::socket_path();
    // Remove stale socket
    let _ = std::fs::remove_file(&socket_path);
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!("Listening on {}", socket_path.display());

    // Signal handler
    let cancel_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Received SIGINT, shutting down");
        cancel_signal.cancel();
    });

    #[cfg(unix)]
    {
        // Register the SIGTERM handler synchronously so a registration failure
        // surfaces as a startup error rather than panicking inside a spawned
        // task and silently losing graceful-shutdown.
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("failed to register SIGTERM handler")?;
        let cancel_term = cancel.clone();
        tokio::spawn(async move {
            sigterm.recv().await;
            tracing::info!("Received SIGTERM, shutting down");
            cancel_term.cancel();
        });
    }

    // Cap concurrent client handler tasks so a flood of connections can't
    // spawn unbounded tasks (roadmap item 9). Excess connections are dropped
    // immediately; a TUI/webui reconnects on its own.
    let client_sem = Arc::new(tokio::sync::Semaphore::new(MAX_CLIENT_TASKS));

    // Main loop
    loop {
        tokio::select! {
            // Accept new client connections
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let permit = match client_sem.clone().try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::warn!(
                                    "client task limit ({MAX_CLIENT_TASKS}) reached; dropping connection"
                                );
                                drop(stream);
                                continue;
                            }
                        };
                        let snapshot = state.snapshot();
                        let client_broadcast_rx = broadcast_tx.subscribe();
                        let rec_tx = recording_tx.clone();
                        let bulk_tx_ref = Some(bulk_tx.clone());
                        let poll_notify = poll_notify.clone();
                        let interval_ctl = interval_ctl.clone();
                        let cancel_ref = cancel.clone();

                        let client_config = config.clone();
                        let client_registry = registry.clone();
                        let client_recordings = state.recordings.clone();
                        let client_event_tx = event_tx.clone();
                        tokio::spawn(async move {
                            // Held for the connection's lifetime; released on drop.
                            let _permit = permit;
                            if let Err(e) = handle_client(
                                stream,
                                snapshot,
                                client_broadcast_rx,
                                rec_tx,
                                bulk_tx_ref,
                                client_config,
                                client_registry,
                                client_recordings,
                                client_event_tx,
                                poll_notify,
                                interval_ctl,
                                cancel_ref,
                            ).await {
                                tracing::debug!("Client disconnected: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::error!("Accept error: {e}");
                    }
                }
            }
            // Process internal events
            Some(event) = event_rx.recv() => {
                if let AppEvent::Daemon(ref de) = event {
                    state.apply(de);
                    // Fan out to all connected clients
                    let _ = broadcast_tx.send(de.clone());

                    // Auto VOD backfill: when a Twitch live recording
                    // ends cleanly, schedule a delayed download of the
                    // archive VOD so we get the first ~5 minutes the
                    // HLS pull missed.
                    if config.recording.auto_vod_backfill {
                        if let DaemonEvent::RecordingFinished {
                            job_id, final_state, ..
                        } = de
                        {
                            if *final_state == crate::recording::job::RecordingState::Finished {
                                if let (Some(job), Some(twitch)) =
                                    (state.recordings.get(job_id), twitch_handle.as_ref())
                                {
                                    if job.platform == PlatformKind::Twitch {
                                        crate::recording::vod_backfill::spawn(
                                            crate::recording::vod_backfill::BackfillRequest {
                                                channel_id: job.channel_id.clone(),
                                                channel_name: job.channel_name.clone(),
                                                started_at: job.started_at,
                                                live_output_path: job.output_path.clone(),
                                                stream_title: job.stream_title.clone(),
                                                delay_secs: config.recording.vod_backfill_delay_secs,
                                            },
                                            twitch.clone(),
                                            recording_tx.clone(),
                                        );
                                    }
                                }
                            }
                        }
                    }

                    // Persist recording lifecycle for crash-recovery audit.
                    if let Some(ref db) = persist_db {
                        let db = db.clone();
                        let de = de.clone();
                        let recordings = state.recordings.clone();
                        tokio::spawn(async move {
                            persist_event(&db, &de, &recordings).await;
                        });
                    }
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("Daemon shutting down");
                break;
            }
        }
    }

    // Cleanup
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&pid_path);
    tracing::info!("StriVo daemon exited");
    Ok(())
}

async fn handle_client(
    stream: tokio::net::UnixStream,
    snapshot: ServerMessage,
    broadcast_rx: broadcast::Receiver<DaemonEvent>,
    recording_tx: mpsc::UnboundedSender<RecordingCommand>,
    bulk_tx: Option<mpsc::UnboundedSender<crate::recording::bulk::BulkCommand>>,
    config: AppConfig,
    registry: Arc<tokio::sync::Mutex<crate::plugin::registry::PluginRegistry>>,
    recordings: HashMap<Uuid, RecordingJob>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    poll_notify: Option<Arc<tokio::sync::Notify>>,
    interval_ctl: Option<(Arc<std::sync::atomic::AtomicU64>, Arc<tokio::sync::Notify>)>,
    cancel: CancellationToken,
) -> Result<()> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    // Note: the first message is NOT required to be Hello. The TUI opens
    // a long-lived connection with Hello (→ snapshot) and then streams
    // commands; the webui's send_command opens a short-lived connection
    // and writes a single command with no Hello. Both are handled in the
    // read loop below (Hello → snapshot via the writer task), so a
    // command-first connection is dispatched rather than dropped.

    // Spawn a writer task that sends broadcast events
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();

    let writer_task = tokio::spawn(async move {
        while let Some(msg) = write_rx.recv().await {
            if writer.write_all(msg.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    // Forward broadcast events
    let write_tx_clone = write_tx.clone();
    let cancel_clone = cancel.clone();
    let mut bcast_rx = broadcast_rx;
    let broadcast_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                result = bcast_rx.recv() => {
                    match result {
                        Ok(event) => {
                            let msg = ServerMessage::Event(event);
                            if let Ok(encoded) = ipc::encode_message(&msg) {
                                if write_tx_clone.send(encoded).is_err() {
                                    break;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            tracing::warn!("Client lagged, they should re-sync via Hello");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = cancel_clone.cancelled() => break,
            }
        }
    });

    // Read client messages
    loop {
        line.clear();
        let n = buf_reader.read_line(&mut line).await?;
        if n == 0 {
            break; // Client disconnected
        }

        let msg: ClientMessage = match serde_json::from_str(line.trim()) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Invalid client message: {e}");
                continue;
            }
        };

        match msg {
            ClientMessage::Hello => {
                // Send the state snapshot through the writer task. (The
                // snapshot is captured at connect time; good enough — the
                // client also receives live events thereafter.)
                if let Ok(encoded) = ipc::encode_message(&snapshot) {
                    let _ = write_tx.send(encoded);
                }
            }
            ClientMessage::Recording(cmd) => {
                let _ = recording_tx.send(cmd);
            }
            ClientMessage::PollNow => {
                if let Some(ref notify) = poll_notify {
                    notify.notify_one();
                }
            }
            ClientMessage::SetPollInterval(secs) => {
                // Live-update the monitor's poll cadence (item 14b). The web
                // endpoint already persisted it to config.toml; here we just
                // apply it to the running monitor.
                let secs = secs.max(15);
                if let Some((ref atomic, ref notify)) = interval_ctl {
                    atomic.store(secs, std::sync::atomic::Ordering::Relaxed);
                    notify.notify_one();
                    tracing::info!("Applied live poll interval: {secs}s");
                }
            }
            ClientMessage::Shutdown => {
                cancel.cancel();
                break;
            }
            ClientMessage::PatreonPull {
                embed_url,
                creator_name,
                post_title,
            } => {
                let output_path = crate::recording::build_output_path(
                    &config,
                    &creator_name,
                    crate::platform::PlatformKind::Patreon,
                    Some(&post_title),
                );
                let _ = recording_tx.send(RecordingCommand::DownloadVod {
                    url: embed_url,
                    channel_name: creator_name,
                    platform: crate::platform::PlatformKind::Patreon,
                    output_path,
                    cookies_path: None,
                    post_title: Some(post_title),
                });
            }
            ClientMessage::ListPlaylists { channel_id } => {
                if let Some(ref tx) = bulk_tx {
                    let _ = tx.send(crate::recording::bulk::BulkCommand::ListPlaylists {
                        channel_id,
                    });
                }
            }
            ClientMessage::FetchChannelVods {
                channel_id,
                platform,
            } => {
                if let Some(ref tx) = bulk_tx {
                    let _ = tx.send(crate::recording::bulk::BulkCommand::FetchVods {
                        channel_id,
                        platform,
                    });
                }
            }
            ClientMessage::ResolveChannel { platform, query } => {
                if let Some(ref tx) = bulk_tx {
                    let _ = tx.send(crate::recording::bulk::BulkCommand::ResolveChannel {
                        platform,
                        query,
                    });
                }
            }
            ClientMessage::BulkDownload {
                channel_id,
                channel_name,
                platform,
                action,
                playlist_id,
            } => {
                let cmd = match action {
                    crate::ipc::BulkAction::Start => {
                        crate::recording::bulk::BulkCommand::Start {
                            channel_id,
                            channel_name,
                            platform,
                            playlist_id,
                        }
                    }
                    crate::ipc::BulkAction::Stop => {
                        crate::recording::bulk::BulkCommand::Stop { channel_id }
                    }
                };
                if let Some(ref tx) = bulk_tx {
                    let _ = tx.send(cmd);
                }
            }
            ClientMessage::PluginRpc {
                plugin,
                verb,
                selection,
                payload: _,
            } => {
                // W2-phase-3 — actually dispatch the verb. The registry
                // is shared (Arc<Mutex>); on_verb takes the narrow
                // VerbContext so we don't need a full AppState. The
                // returned PluginActions are processed headless: the
                // SpawnTask futures (the real work — transcription,
                // archive pulls) are spawned, and SetStatus/Notify are
                // surfaced as daemon notifications. TUI-only actions
                // (ActivatePane/NavigateBack) are no-ops here.
                let actions = {
                    let mut reg = registry.lock().await;
                    let ctx = crate::plugin::VerbContext {
                        recordings: &recordings,
                    };
                    reg.dispatch_verb(&plugin, &verb, &selection, &ctx)
                };
                tracing::info!(
                    plugin = %plugin,
                    verb = %verb,
                    action_count = actions.len(),
                    "daemon: dispatched plugin verb"
                );
                process_daemon_plugin_actions(actions, &registry, &event_tx);
            }
        }
    }

    broadcast_task.abort();
    writer_task.abort();
    Ok(())
}

/// Process PluginActions returned by a daemon-side verb dispatch (W2-phase-3).
/// Headless: the SpawnTask futures (the real work) are spawned, and their
/// follow-up plugin events are pumped back through the shared registry so a
/// multi-stage verb runs to completion. SetStatus/Notify become daemon
/// notifications (visible to connected TUI/web clients). TUI-only actions
/// (pane activation, mpv playback) are no-ops in the daemon.
fn process_daemon_plugin_actions(
    actions: Vec<crate::plugin::PluginAction>,
    registry: &Arc<tokio::sync::Mutex<crate::plugin::registry::PluginRegistry>>,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    use crate::plugin::PluginAction as PA;
    for action in actions {
        match action {
            PA::SetStatus(s) => {
                let _ = event_tx.send(AppEvent::notification("Plugin".to_string(), s));
            }
            PA::Notify { title, body } => {
                let _ = event_tx.send(AppEvent::notification(title, body));
            }
            PA::SpawnTask {
                plugin_name,
                future,
            } => {
                let reg = registry.clone();
                let etx = event_tx.clone();
                tokio::spawn(async move {
                    let result = future.await;
                    let next = {
                        let mut r = reg.lock().await;
                        r.dispatch_plugin_event(plugin_name, result)
                    };
                    // Recurse: the follow-up actions may spawn further
                    // stages (e.g. transcription pipeline steps).
                    process_daemon_plugin_actions(next, &reg, &etx);
                });
            }
            // No daemon equivalent (no TUI panes / mpv / config persistence
            // path here); the TUI handles these when it dispatches verbs.
            _ => {}
        }
    }
}

/// Persist a recording's lifecycle for crash-recovery. Best-effort — a sqlite
/// hiccup never breaks the live event flow.
async fn persist_event(
    db: &crate::recording::persist::PersistDb,
    event: &DaemonEvent,
    recordings: &HashMap<Uuid, RecordingJob>,
) {
    use crate::recording::job::RecordingState;
    use crate::recording::persist::PersistedJob;

    // Encode a RecordingJob to JSON for the journal. Serialization is not
    // expected to fail for our types, but if it does we record a structured
    // error marker so the persisted row remains diagnostically useful rather
    // than silently empty.
    fn encode_job(job: &RecordingJob) -> String {
        match serde_json::to_string(job) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(job_id = %job.id, "daemon: failed to serialize recording job for journal: {e}");
                format!(
                    r#"{{"_serialize_error":"{}","job_id":"{}"}}"#,
                    e.to_string().replace('"', "'"),
                    job.id
                )
            }
        }
    }

    let snapshot =
        |job_id: &Uuid, state: RecordingState, error: Option<String>| -> Option<PersistedJob> {
            let job = recordings.get(job_id)?;
            Some(PersistedJob {
                id: job.id.to_string(),
                kind: "Recording".to_string(),
                payload: encode_job(job),
                state: format!("{state:?}").to_lowercase(),
                attempts: 0,
                last_error: error,
                episode_dir: job.output_path.parent().map(|p| p.to_path_buf()),
            })
        };

    let result = match event {
        DaemonEvent::RecordingStarted { job } => {
            let pj = PersistedJob {
                id: job.id.to_string(),
                kind: "Recording".to_string(),
                payload: encode_job(job),
                state: "running".to_string(),
                attempts: 0,
                last_error: None,
                episode_dir: job.output_path.parent().map(|p| p.to_path_buf()),
            };
            db.upsert_job(&pj).await
        }
        DaemonEvent::RecordingFinished {
            job_id,
            final_state,
            error,
        } => {
            let Some(pj) = snapshot(job_id, *final_state, error.clone()) else {
                return;
            };
            db.upsert_job(&pj).await
        }
        _ => return,
    };
    if let Err(e) = result {
        tracing::warn!("daemon: persist event failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::job::{RecordingJob, RecordingState};
    use crate::platform::PlatformKind;

    fn empty_state() -> DaemonState {
        DaemonState {
            channels: Vec::new(),
            recordings: HashMap::new(),
            twitch_connected: false,
            youtube_connected: false,
            patreon_connected: false,
            pending_auth: None,
            auth_queue: std::collections::VecDeque::new(),
            patreon_creators: Vec::new(),
            patreon_posts: Vec::new(),
        }
    }

    fn job(state: RecordingState, age_secs: i64) -> RecordingJob {
        let mut j = RecordingJob::new(
            "ch".into(),
            "Chan".into(),
            PlatformKind::Twitch,
            std::path::PathBuf::from("/tmp/x.mkv"),
            false,
            None,
        );
        j.state = state;
        j.started_at = chrono::Utc::now() - chrono::Duration::seconds(age_secs);
        j
    }

    #[test]
    fn evict_caps_terminal_keeps_active() {
        let mut st = empty_state();
        // One active job (must survive) plus a terminal job older than any
        // we add below, to confirm the oldest terminal is the one dropped.
        let active = job(RecordingState::Recording, 1);
        let active_id = active.id;
        st.recordings.insert(active_id, active);
        let oldest = job(RecordingState::Finished, 1_000_000);
        let oldest_id = oldest.id;
        st.recordings.insert(oldest_id, oldest);

        // Push terminal jobs well past the cap.
        for i in 0..MAX_TERMINAL_RECORDINGS + 50 {
            let j = job(RecordingState::Finished, i as i64);
            st.recordings.insert(j.id, j);
        }

        st.evict_old_terminal();

        let terminal = st
            .recordings
            .values()
            .filter(|j| matches!(j.state, RecordingState::Finished | RecordingState::Failed))
            .count();
        assert_eq!(terminal, MAX_TERMINAL_RECORDINGS, "terminal tail capped");
        assert!(st.recordings.contains_key(&active_id), "active job kept");
        assert!(!st.recordings.contains_key(&oldest_id), "oldest terminal dropped");
    }

    #[test]
    fn evict_noop_under_cap() {
        let mut st = empty_state();
        for i in 0..10 {
            let j = job(RecordingState::Finished, i);
            st.recordings.insert(j.id, j);
        }
        st.evict_old_terminal();
        assert_eq!(st.recordings.len(), 10);
    }
}
