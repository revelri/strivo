use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
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

/// Daemon state — maintained from internal events.
struct DaemonState {
    channels: Vec<ChannelEntry>,
    recordings: HashMap<Uuid, RecordingJob>,
    twitch_connected: bool,
    youtube_connected: bool,
    patreon_connected: bool,
    pending_auth: Option<(PlatformKind, String, String)>,
    auth_queue: std::collections::VecDeque<(PlatformKind, String, String)>,
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
        }
    }

    fn apply(&mut self, event: &DaemonEvent) {
        match event {
            DaemonEvent::ChannelsUpdated(channels) => {
                self.channels = channels.clone();
            }
            DaemonEvent::RecordingStarted { job } => {
                self.recordings.insert(job.id, job.clone());
            }
            DaemonEvent::RecordingProgress { job_id, bytes_written, duration_secs } => {
                if let Some(job) = self.recordings.get_mut(job_id) {
                    job.bytes_written = *bytes_written;
                    job.duration_secs = *duration_secs;
                    job.state = crate::recording::job::RecordingState::Recording;
                }
            }
            DaemonEvent::RecordingFinished { job_id, final_state, error } => {
                if let Some(job) = self.recordings.get_mut(job_id) {
                    job.state = *final_state;
                    job.error = error.clone();
                }
            }
            DaemonEvent::DeviceCodeRequired { kind, verification_uri, user_code } => {
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

pub async fn run() -> Result<()> {
    // Initialize logging
    let state_dir = AppConfig::state_dir();
    std::fs::create_dir_all(&state_dir)?;

    let log_path = state_dir.join("strivo.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    tracing::info!("StriVo daemon starting");

    // Write PID file
    let pid_path = ipc::pid_path();
    std::fs::write(&pid_path, std::process::id().to_string())?;

    // Validate external tools
    crate::check_external_tools();

    // Load config
    let config = AppConfig::load(None)?;
    tracing::info!("Config loaded");

    // Open the persistence db (jobs / catalog / crunchr_queue) and recover any
    // jobs that were marked running when the daemon last died. Recovery is
    // intentionally minimal: we mark orphans as 'interrupted' so the audit log
    // is honest. Catalog-pull resumption is automatic — the catalog dedupe
    // index in §5 already skips already-recorded VODs on the next pull.
    let persist_db = match crate::recording::persist::PersistDb::open(&AppConfig::data_dir().join("jobs.db")) {
        Ok(db) => {
            match db.recover_orphaned_running().await {
                Ok(n) if n > 0 => tracing::info!("daemon: marked {n} orphan job(s) as interrupted"),
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

    if let Some(ref twitch_config) = config.twitch {
        let mut twitch = crate::platform::twitch::TwitchPlatform::new(
            twitch_config.client_id.clone(),
            twitch_config.client_secret.clone(),
        );
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
    tokio::spawn(async move {
        crate::recording::run_manager(rec_config, recording_rx, rec_tx, rec_cancel).await;
    });

    // Spawn channel monitor
    let poll_notify = if !platforms.is_empty() {
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
        let cancel_term = cancel.clone();
        tokio::spawn(async move {
            let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Failed to register SIGTERM");
            sig.recv().await;
            tracing::info!("Received SIGTERM, shutting down");
            cancel_term.cancel();
        });
    }

    // Main loop
    loop {
        tokio::select! {
            // Accept new client connections
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let snapshot = state.snapshot();
                        let client_broadcast_rx = broadcast_tx.subscribe();
                        let rec_tx = recording_tx.clone();
                        let poll_notify = poll_notify.clone();
                        let cancel_ref = cancel.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_client(
                                stream,
                                snapshot,
                                client_broadcast_rx,
                                rec_tx,
                                poll_notify,
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
    poll_notify: Option<Arc<tokio::sync::Notify>>,
    cancel: CancellationToken,
) -> Result<()> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    // Read Hello
    line.clear();
    buf_reader.read_line(&mut line).await?;
    let msg: ClientMessage = serde_json::from_str(line.trim())?;
    match msg {
        ClientMessage::Hello => {
            let encoded = ipc::encode_message(&snapshot)?;
            writer.write_all(encoded.as_bytes()).await?;
        }
        _ => {
            anyhow::bail!("Expected Hello, got {:?}", msg);
        }
    }

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
                // Re-sync: send fresh snapshot would require state access
                // For now just acknowledge
                tracing::debug!("Client re-sent Hello");
            }
            ClientMessage::Recording(cmd) => {
                let _ = recording_tx.send(cmd);
            }
            ClientMessage::PollNow => {
                if let Some(ref notify) = poll_notify {
                    notify.notify_one();
                }
            }
            ClientMessage::Shutdown => {
                cancel.cancel();
                break;
            }
        }
    }

    broadcast_task.abort();
    writer_task.abort();
    Ok(())
}

/// Persist a recording's lifecycle for crash-recovery. Best-effort — a sqlite
/// hiccup never breaks the live event flow.
async fn persist_event(
    db: &crate::recording::persist::PersistDb,
    event: &DaemonEvent,
    recordings: &HashMap<Uuid, RecordingJob>,
) {
    use crate::recording::persist::PersistedJob;
    use crate::recording::job::RecordingState;

    let snapshot = |job_id: &Uuid, state: RecordingState, error: Option<String>| -> Option<PersistedJob> {
        let job = recordings.get(job_id)?;
        Some(PersistedJob {
            id: job.id.to_string(),
            kind: "Recording".to_string(),
            payload: serde_json::to_string(job).unwrap_or_default(),
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
                payload: serde_json::to_string(job).unwrap_or_default(),
                state: "running".to_string(),
                attempts: 0,
                last_error: None,
                episode_dir: job.output_path.parent().map(|p| p.to_path_buf()),
            };
            db.upsert_job(&pj).await
        }
        DaemonEvent::RecordingFinished { job_id, final_state, error } => {
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
