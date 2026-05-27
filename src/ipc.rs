use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::DaemonEvent;
use crate::platform::{ChannelEntry, PlatformKind};
use crate::recording::job::RecordingJob;
use crate::recording::RecordingCommand;

/// Messages sent from TUI client to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Request full state snapshot
    Hello,
    /// Forward a recording command
    Recording(RecordingCommand),
    /// Trigger immediate channel poll
    PollNow,
    /// Live-update the channel-poll interval without a restart (item 14b).
    /// Seconds; the daemon clamps to a sane minimum.
    SetPollInterval(u64),
    /// Graceful daemon shutdown
    Shutdown,
    /// Dispatch an actions-popup verb to a plugin via the host
    /// `PluginRegistry::dispatch_verb`. (Part 11 W2.)
    PluginRpc {
        plugin: String,
        verb: String,
        /// Recording UUIDs the verb should act on (selection set in
        /// the TUI; cursor row in single-select).
        #[serde(default)]
        selection: Vec<Uuid>,
        /// Optional JSON payload for plugin-specific args. The
        /// plugin parses or ignores; the host doesn't inspect it.
        #[serde(default)]
        payload: serde_json::Value,
    },
    /// Start or stop a per-channel bulk back-catalog download (task #71).
    BulkDownload {
        channel_id: String,
        channel_name: String,
        platform: PlatformKind,
        action: BulkAction,
        /// Optional YouTube playlist scope (task #73). None = whole channel.
        #[serde(default)]
        playlist_id: Option<String>,
    },
    /// Request the playlists for a YouTube channel, to populate the
    /// bulk-download scope picker (task #73). Answered asynchronously
    /// with DaemonEvent::PlaylistList.
    ListPlaylists {
        channel_id: String,
    },
    /// Pull a single Patreon video post on demand (task #75 — webui
    /// equivalent of the TUI's PullPatreonPost). The daemon builds the
    /// output path from its config, so the webui doesn't have to.
    PatreonPull {
        embed_url: String,
        creator_name: String,
        post_title: String,
    },
    /// Request a channel's recent VODs (live broadcasts + uploads) for the
    /// webui channel-detail pane. Answered asynchronously with
    /// DaemonEvent::ChannelVods.
    FetchChannelVods {
        channel_id: String,
        platform: PlatformKind,
    },
    /// Resolve a human-entered identifier (Twitch login, YouTube/Patreon id)
    /// to a channel id for the Add-Channel wizard (task #19). Answered
    /// asynchronously with DaemonEvent::ChannelResolved.
    ResolveChannel {
        platform: PlatformKind,
        query: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BulkAction {
    Start,
    Stop,
}

/// Messages sent from daemon to TUI client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Full state snapshot (sent in response to Hello)
    StateSnapshot {
        channels: Vec<ChannelEntry>,
        recordings: HashMap<Uuid, RecordingJob>,
        twitch_connected: bool,
        youtube_connected: bool,
        patreon_connected: bool,
        pending_auth: Option<(PlatformKind, String, String)>,
        /// Latest Patreon snapshot (creators + their video posts), cached
        /// from the most recent PatreonState event so a client connecting
        /// between polls sees Patreon immediately instead of waiting up to
        /// a full poll interval. Defaults empty for older clients.
        #[serde(default)]
        patreon_creators: Vec<ChannelEntry>,
        #[serde(default)]
        patreon_posts: Vec<crate::platform::patreon::PatreonPost>,
    },
    /// Incremental update event
    Event(DaemonEvent),
}

/// Socket path for the daemon
pub fn socket_path() -> std::path::PathBuf {
    crate::config::AppConfig::state_dir().join("strivo.sock")
}

/// PID file path for the daemon
pub fn pid_path() -> std::path::PathBuf {
    crate::config::AppConfig::state_dir().join("strivo.pid")
}

/// Write a message as newline-delimited JSON
pub fn encode_message<T: Serialize>(msg: &T) -> Result<String, serde_json::Error> {
    let mut s = serde_json::to_string(msg)?;
    s.push('\n');
    Ok(s)
}

/// Check if the daemon is running.
///
/// `kill(pid, 0)` alone can produce false positives: PIDs get recycled
/// after a crash, and the recorded PID may belong to an unrelated
/// process. Before trusting the PID we also confirm the Unix socket is
/// bound *and* still accepting connections. A blocking connect with a
/// ~200 ms budget is the cheapest definitive liveness probe; a dead
/// socket rejects `connect(2)` with `ECONNREFUSED` almost instantly.
pub fn is_daemon_running() -> bool {
    let pid_file = pid_path();
    let sock_file = socket_path();
    if !pid_file.exists() || !sock_file.exists() {
        return false;
    }
    let Ok(pid_str) = std::fs::read_to_string(&pid_file) else {
        return false;
    };
    let Ok(pid) = pid_str.trim().parse::<i32>() else {
        return false;
    };
    // Safety: kill(pid, 0) is the canonical reachability probe — no signal
    // is delivered.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    // Cross-check: actually connect to the socket. If the daemon crashed
    // and the PID got recycled, the socket file may still sit on disk
    // but nothing is accept(2)ing on it.
    match std::os::unix::net::UnixStream::connect(&sock_file) {
        Ok(stream) => {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(200)));
            true
        }
        Err(_) => false,
    }
}

/// Remove stale pid + socket files left by a previous daemon that
/// crashed. Safe to call at the start of every `daemon` command.
pub fn sweep_stale_files() {
    let pid_file = pid_path();
    let sock_file = socket_path();
    let stale_pid = match std::fs::read_to_string(&pid_file) {
        Ok(s) => match s.trim().parse::<i32>() {
            Ok(pid) => unsafe { libc::kill(pid, 0) != 0 },
            Err(_) => true,
        },
        Err(_) => !pid_file.exists(),
    };
    if stale_pid {
        let _ = std::fs::remove_file(&pid_file);
        let _ = std::fs::remove_file(&sock_file);
    }
}
