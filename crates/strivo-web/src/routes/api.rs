//! /api/v1/* JSON surface (webui phase 9).
//!
//! Read-only endpoints today — every consumer (Tdarr, Bazarr, custom
//! automations) needs them, and writes are scoped to the HTML form
//! handlers until we've vetted a CSRF posture.
//!
//! Auth: `X-Api-Key: <key>` header. Constant-time compare via
//! `auth::ApiKey::matches`.
//!
//! Endpoints:
//!   GET /api/v1/health           — liveness probe, no auth required
//!   GET /api/v1/channels         — snapshot of every monitored channel
//!   GET /api/v1/recordings       — snapshot of every recording
//!   GET /api/v1/recordings/<id>  — single recording
//!   GET /api/v1/schedule         — config.schedule entries
//!   GET /api/v1/settings         — non-secret config snapshot

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use strivo_core::ipc::{BulkAction, ClientMessage, ServerMessage};
use strivo_core::platform::PlatformKind;
use strivo_core::recording::RecordingCommand;
use uuid::Uuid;

use crate::server::AppState;

/// Authorize a request via EITHER the `X-Api-Key` header (programmatic
/// clients) OR a valid `strivo_session` cookie (browser, set by /login).
/// The browser SPA only carries the cookie, so cookie support is what
/// lets it reach /channels, /recordings, … after logging in. (W3.)
fn check_key(headers: &HeaderMap, state: &AppState) -> Result<(), StatusCode> {
    // Single dual-track gate: valid X-Api-Key header OR a valid session
    // cookie (plain or `__Host-` name). See login::check_dual.
    crate::routes::login::check_dual(headers, &state.api_key, &state.session_secret)
}

async fn channels(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state.ipc.snapshot().await {
        Ok(ServerMessage::StateSnapshot { channels, .. }) => {
            Json(json!({ "channels": channels })).into_response()
        }
        Ok(_) => Json(json!({ "channels": [] })).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

/// `GET /api/v1/patreon` — current Patreon creators + their video posts,
/// cached in the daemon snapshot so the SPA can seed its Patreon section
/// on load instead of waiting up to a full poll interval for the next
/// patreon-state SSE event.
async fn patreon(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state.ipc.snapshot().await {
        Ok(ServerMessage::StateSnapshot {
            patreon_creators,
            patreon_posts,
            ..
        }) => Json(json!({ "creators": patreon_creators, "posts": patreon_posts }))
            .into_response(),
        Ok(_) => Json(json!({ "creators": [], "posts": [] })).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

async fn recordings(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state.ipc.snapshot().await {
        Ok(ServerMessage::StateSnapshot { recordings, .. }) => {
            // recordings is a HashMap<Uuid, RecordingJob>; flatten to a list
            // so the response is stable and order-by-newest-first.
            let mut items: Vec<_> = recordings.into_values().collect();
            items.sort_by_key(|r| std::cmp::Reverse(r.started_at));
            Json(json!({ "recordings": items })).into_response()
        }
        Ok(_) => Json(json!({ "recordings": [] })).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

async fn recording_one(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state.ipc.snapshot().await {
        Ok(ServerMessage::StateSnapshot { recordings, .. }) => match recordings.get(&id) {
            Some(j) => Json(j.clone()).into_response(),
            None => crate::problem::Problem::not_found("recording not found").into_response(),
        },
        Ok(_) => crate::problem::Problem::internal("unexpected response").into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

async fn schedule(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match strivo_core::config::AppConfig::load(None) {
        Ok(cfg) => {
            // Annotate each entry with its next fire time (RFC3339) so the
            // webui "Upcoming" row can sort + display it. Mirrors the cron
            // parse in src/recording/schedule.rs (5-field → prepend "0 ").
            let entries: Vec<_> = cfg
                .schedule
                .iter()
                .map(|e| {
                    let cron_expr = if e.cron.split_whitespace().count() == 5 {
                        format!("0 {}", e.cron)
                    } else {
                        e.cron.clone()
                    };
                    let next_fire = std::str::FromStr::from_str(&cron_expr)
                        .ok()
                        .and_then(|s: cron::Schedule| s.upcoming(chrono::Utc).next())
                        .map(|dt| dt.to_rfc3339());
                    json!({
                        "channel": e.channel,
                        "cron": e.cron,
                        "duration": e.duration,
                        "next_fire": next_fire,
                    })
                })
                .collect();
            Json(json!({ "schedule": entries })).into_response()
        }
        Err(e) => crate::problem::Problem::internal(e.to_string()).into_response(),
    }
}

async fn settings(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match strivo_core::config::AppConfig::load(None) {
        Ok(cfg) => {
            // Strip secrets — never expose client_secret / cookies_path.
            // We surface only the existence (`configured: bool`) of each
            // platform, not their credential payloads.
            let body = json!({
                "recording_dir": cfg.recording_dir,
                "poll_interval_secs": cfg.poll_interval_secs,
                "recording": cfg.recording,
                "ui": cfg.ui,
                "auto_record_channels": cfg.auto_record_channels,
                "capture_profiles": cfg.capture_profiles,
                "schedule": cfg.schedule,
                "archiver": cfg.archiver,
                "twitch_configured": cfg.twitch.is_some(),
                "youtube_configured": cfg.youtube.is_some(),
                "patreon_configured": cfg.patreon.is_some(),
            });
            Json(body).into_response()
        }
        Err(e) => crate::problem::Problem::internal(e.to_string()).into_response(),
    }
}

/// `GET /api/v1/health` — machine-readable health for CI/monitoring
/// (roadmap item 11). Unauthenticated liveness+readiness: probes the daemon
/// (IPC snapshot round-trip), the jobs DB (open), and free disk on the
/// recording filesystem. 200 when all pass, 503 when any check is degraded,
/// so a monitor can alert on the status code alone.
async fn health(State(state): State<AppState>) -> impl IntoResponse {
    // Daemon reachable: a successful snapshot proves the recorder process is
    // alive and answering on the IPC socket.
    let daemon_ok = state.ipc.snapshot().await.is_ok();

    // Jobs DB openable.
    let db_path = strivo_core::config::AppConfig::data_dir().join("jobs.db");
    let db_ok = strivo_core::recording::persist::PersistDb::open(&db_path).is_ok();

    // Free disk on the recording filesystem.
    let (disk, disk_ok) = match strivo_core::config::AppConfig::load(None) {
        Ok(cfg) => {
            let (total, avail) = statvfs_bytes(&cfg.recording_dir).unwrap_or((0, 0));
            (
                json!({
                    "recording_dir": cfg.recording_dir,
                    "filesystem_total_bytes": total,
                    "filesystem_avail_bytes": avail,
                }),
                avail > 0,
            )
        }
        Err(_) => (serde_json::Value::Null, false),
    };

    let ok = daemon_ok && db_ok && disk_ok;
    let body = json!({
        "status": if ok { "ok" } else { "degraded" },
        "version": env!("CARGO_PKG_VERSION"),
        "checks": {
            "daemon": if daemon_ok { "ok" } else { "unreachable" },
            "db": if db_ok { "ok" } else { "error" },
            "disk": if disk_ok { "ok" } else { "warn" },
        },
        "disk": disk,
    });
    let code = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(body))
}

fn human_bytes(b: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    if b >= GB {
        format!("{:.1} GB", b as f64 / GB as f64)
    } else if b >= MB {
        format!("{:.0} MB", b as f64 / MB as f64)
    } else {
        format!("{b} B")
    }
}

/// Disk free-space severity for the recording filesystem (roadmap item 13).
/// <5% free = error, <15% = warn, else ok; unknown (total 0) = warn.
fn disk_severity(total: u64, avail: u64) -> &'static str {
    if total == 0 {
        return "warn";
    }
    let frac = avail as f64 / total as f64;
    if frac < 0.05 {
        "error"
    } else if frac < 0.15 {
        "warn"
    } else {
        "ok"
    }
}

fn add_check(checks: &mut Vec<serde_json::Value>, domain: &str, name: &str, sev: &str, message: String, fix: &str) {
    checks.push(json!({
        "domain": domain, "name": name, "severity": sev,
        "message": message, "fix": fix,
    }));
}

/// `GET /api/v1/health/checks` — grouped, retestable health checks for the
/// System page (roadmap item 13). Each check carries {domain, name,
/// severity (ok|warn|error), message, fix}; overall status is the worst
/// severity. Authenticated (unlike the plain `/health` liveness probe).
async fn health_checks(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let mut checks: Vec<serde_json::Value> = Vec::new();

    // Network — daemon reachable, and the platform-connected flags it reports.
    let snap = state.ipc.snapshot().await;
    let (tw_c, yt_c, pt_c) = match &snap {
        Ok(ServerMessage::StateSnapshot {
            twitch_connected,
            youtube_connected,
            patreon_connected,
            ..
        }) => (*twitch_connected, *youtube_connected, *patreon_connected),
        _ => (false, false, false),
    };
    if snap.is_ok() {
        add_check(&mut checks, "Network", "Daemon IPC", "ok", "Daemon reachable.".into(), "");
    } else {
        add_check(&mut checks, "Network", "Daemon IPC", "error",
            "Daemon unreachable over the IPC socket.".into(),
            "Start the daemon (strivo daemon) or check the socket.");
    }

    // Storage — free space on the recording filesystem.
    let cfg = strivo_core::config::AppConfig::load(None);
    match &cfg {
        Ok(cfg) => {
            let (total, avail) = statvfs_bytes(&cfg.recording_dir).unwrap_or((0, 0));
            let sev = disk_severity(total, avail);
            let msg = if total == 0 {
                format!("Cannot stat recording dir {}.", cfg.recording_dir.display())
            } else {
                format!("{} free of {}.", human_bytes(avail), human_bytes(total))
            };
            let fix = if sev == "ok" { "" } else { "Free space or change recording_dir." };
            add_check(&mut checks, "Storage", "Disk space", sev, msg, fix);
        }
        Err(e) => add_check(&mut checks, "Storage", "Config", "error",
            format!("config load failed: {e}"), "Fix config.toml."),
    }

    // Platform Auth — configured-but-not-authenticated is a warning.
    if let Ok(cfg) = &cfg {
        for (name, configured, connected) in [
            ("Twitch", cfg.twitch.is_some(), tw_c),
            ("YouTube", cfg.youtube.is_some(), yt_c),
            ("Patreon", cfg.patreon.is_some(), pt_c),
        ] {
            if configured && connected {
                add_check(&mut checks, "Platform Auth", name, "ok", format!("{name} authenticated."), "");
            } else if configured {
                add_check(&mut checks, "Platform Auth", name, "warn",
                    format!("{name} configured but not authenticated."),
                    "Authenticate the platform (TUI login or re-run auth).");
            }
        }
    }

    let worst = if checks.iter().any(|c| c["severity"] == "error") {
        "error"
    } else if checks.iter().any(|c| c["severity"] == "warn") {
        "warn"
    } else {
        "ok"
    };
    Json(json!({ "status": worst, "checks": checks })).into_response()
}

/// `GET /api/v1/storage` — disk usage of the recording directory.
/// (W5 — storage gauges.) Returns bytes_used + bytes_free for the
/// filesystem the recording dir lives on, plus per-platform totals
/// computed by walking the recording-dir tree.
async fn storage(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let cfg = match strivo_core::config::AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => {
            return crate::problem::Problem::internal(e.to_string()).into_response();
        }
    };
    let path = cfg.recording_dir.clone();
    let total_used = walk_dir_bytes(&path).unwrap_or(0);
    // Filesystem stats via statvfs — bytes_free is the more useful
    // signal than bytes_total for "do I have room for the next
    // recording".
    let (fs_total, fs_avail) = statvfs_bytes(&path).unwrap_or((0, 0));
    Json(json!({
        "recording_dir": path,
        "bytes_used_by_recordings": total_used,
        "filesystem_total_bytes": fs_total,
        "filesystem_avail_bytes": fs_avail,
    }))
    .into_response()
}

fn walk_dir_bytes(p: &std::path::Path) -> std::io::Result<u64> {
    let mut sum: u64 = 0;
    if !p.exists() {
        return Ok(0);
    }
    for entry in std::fs::read_dir(p)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            sum = sum.saturating_add(walk_dir_bytes(&entry.path()).unwrap_or(0));
        } else if meta.is_file() {
            sum = sum.saturating_add(meta.len());
        }
    }
    Ok(sum)
}

#[cfg(unix)]
fn statvfs_bytes(p: &std::path::Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let c_path = CString::new(p.to_str()?).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    let block_size = stat.f_frsize as u64;
    let total = stat.f_blocks as u64 * block_size;
    let avail = stat.f_bavail as u64 * block_size;
    Some((total, avail))
}

#[cfg(not(unix))]
fn statvfs_bytes(_p: &std::path::Path) -> Option<(u64, u64)> {
    None
}

/// `GET /api/v1/gantt?hours=24` — recordings as Gantt segments for
/// the dashboard's 24h timeline. Returns
/// `[{ id, channel_name, start_at, end_at, state }, …]`. (W5.)
async fn gantt(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state.ipc.snapshot().await {
        Ok(ServerMessage::StateSnapshot { recordings, .. }) => {
            let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);
            let mut items: Vec<_> = recordings
                .values()
                .filter(|r| r.started_at > cutoff)
                .map(|r| {
                    // RecordingJob has no separate ended_at field;
                    // compute end as start + duration_secs (live
                    // recordings track duration too). For active
                    // jobs that's "now-ish" — close enough for a
                    // 24h Gantt.
                    let end = r.started_at
                        + chrono::Duration::milliseconds((r.duration_secs * 1000.0) as i64);
                    json!({
                        "id": r.id,
                        "channel_name": r.channel_name,
                        "platform": r.platform,
                        "stream_title": r.stream_title,
                        "start_at": r.started_at,
                        "end_at": end,
                        "state": format!("{:?}", r.state),
                        "bytes_written": r.bytes_written,
                    })
                })
                .collect();
            items.sort_by(|a, b| a["start_at"].to_string().cmp(&b["start_at"].to_string()));
            Json(json!({ "window_hours": 24, "items": items })).into_response()
        }
        _ => Json(json!({ "window_hours": 24, "items": [] })).into_response(),
    }
}

// ── W1: mutation endpoints ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct StartRecordingPayload {
    channel_id: String,
    channel_name: String,
    #[serde(default)]
    display_name: Option<String>,
    platform: PlatformKind,
    #[serde(default)]
    transcode: bool,
    #[serde(default)]
    from_start: bool,
    #[serde(default)]
    stream_title: Option<String>,
}

/// `POST /api/v1/recordings` — start a new recording. Equivalent to
/// the TUI's `r` / `R` keys on the Detail pane. (W1.)
async fn start_recording(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<StartRecordingPayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let cmd = ClientMessage::Recording(RecordingCommand::Start {
        channel_id: body.channel_id,
        channel_name: body.channel_name,
        display_name: body.display_name,
        platform: body.platform,
        transcode: body.transcode,
        cookies_path: None,
        stream_title: body.stream_title,
        from_start: body.from_start,
        job_id: None,
    });
    match state.ipc.send_command(cmd).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "queued"}))).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

/// `DELETE /api/v1/recordings/<id>` — stop the recording with the
/// given job id. Equivalent to the TUI's `s` on a RecordingList row. (W1.)
async fn stop_recording(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let cmd = ClientMessage::Recording(RecordingCommand::Stop { job_id: id });
    match state.ipc.send_command(cmd).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({"status": "stop sent", "job_id": id})),
        )
            .into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

/// `POST /api/v1/recordings/stop_all` — equivalent to the TUI's quit
/// confirmation flow when active recordings are running.
async fn stop_all_recordings(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state.ipc.send_command(ClientMessage::Recording(RecordingCommand::StopAll)).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "stop_all sent"}))).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

/// `POST /api/v1/poll_now` — pokes the channel monitor (TUI sends
/// `ClientMessage::PollNow` via the same path).
async fn poll_now(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state.ipc.send_command(ClientMessage::PollNow).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "polled"}))).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct PollIntervalPayload {
    secs: u64,
}

/// `POST /api/v1/settings/poll_interval` — persist a new channel-poll interval
/// to config.toml AND apply it live to the running daemon (item 14b). Clamped
/// to a 15s floor (matching the monitor).
async fn set_poll_interval(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<PollIntervalPayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let secs = body.secs.max(15);
    // Persist to config.toml so the change survives a restart.
    match strivo_core::config::AppConfig::load(None) {
        Ok(mut cfg) => {
            cfg.poll_interval_secs = secs;
            let path = cfg.config_path.clone();
            if let Err(e) = cfg.save(path.as_deref()) {
                return crate::problem::Problem::internal(format!("save config: {e}")).into_response();
            }
        }
        Err(e) => return crate::problem::Problem::internal(e.to_string()).into_response(),
    }
    // Apply live to the running monitor.
    match state.ipc.send_command(ClientMessage::SetPollInterval(secs)).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"poll_interval_secs": secs}))).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

/// Numeric severity for a `tracing` level token, higher = more severe.
fn level_rank(level: &str) -> u8 {
    match level.to_ascii_uppercase().as_str() {
        "ERROR" => 4,
        "WARN" => 3,
        "INFO" => 2,
        "DEBUG" => 1,
        "TRACE" => 0,
        _ => 2,
    }
}

/// Extract the level token from a `tracing` fmt line, e.g.
/// `2026-05-26T18:00:00Z  INFO strivo::x: msg` → `INFO`.
fn line_level(line: &str) -> Option<&str> {
    line.split_whitespace()
        .find(|t| matches!(*t, "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE"))
}

/// Newest `strivo.<date>.log` in the state dir (rolling appender output).
fn newest_log_file(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("strivo") && n.ends_with(".log"))
        })
        .max_by_key(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
        })
}

#[derive(Debug, Deserialize)]
struct LogQuery {
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    lines: Option<usize>,
}

/// `GET /api/v1/logs?level=info&lines=200` — tail the newest rolling log
/// file, filtered to the given minimum level (roadmap item 15). Bounded by
/// `lines` (default 200, max 2000) so users never SSH for logs.
async fn logs(
    headers: HeaderMap,
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<LogQuery>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let dir = strivo_core::config::AppConfig::state_dir();
    let Some(path) = newest_log_file(&dir) else {
        return Json(json!({ "lines": [], "level": "info", "file": null })).into_response();
    };
    let body = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) => return crate::problem::Problem::internal(e.to_string()).into_response(),
    };
    let min = level_rank(q.level.as_deref().unwrap_or("trace"));
    let cap = q.lines.unwrap_or(200).clamp(1, 2000);
    let mut filtered: Vec<&str> = body
        .lines()
        .filter(|l| line_level(l).map_or(true, |lv| level_rank(lv) >= min))
        .collect();
    if filtered.len() > cap {
        filtered = filtered.split_off(filtered.len() - cap);
    }
    Json(json!({
        "lines": filtered,
        "level": q.level.unwrap_or_else(|| "trace".into()),
        "file": path.file_name().and_then(|n| n.to_str()),
    }))
    .into_response()
}

// ── Config/DB backup + restore (roadmap item 16) ─────────────────────
// Dep-free: a backup is a directory `data_dir/backups/<timestamp>/`
// holding copies of config.toml + jobs.db.

fn backups_dir() -> std::path::PathBuf {
    strivo_core::config::AppConfig::data_dir().join("backups")
}

/// Validate a user-supplied backup name: plain filename, no path parts.
/// Prevents traversal on the restore/download path.
fn safe_backup_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

async fn backup_create(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let name = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let dest = backups_dir().join(&name);
    if let Err(e) = std::fs::create_dir_all(&dest) {
        return crate::problem::Problem::internal(format!("create backup dir: {e}")).into_response();
    }
    let cfg = strivo_core::config::AppConfig::config_path();
    let db = strivo_core::config::AppConfig::data_dir().join("jobs.db");
    let mut copied = Vec::new();
    if cfg.exists() {
        if let Err(e) = std::fs::copy(&cfg, dest.join("config.toml")) {
            return crate::problem::Problem::internal(format!("copy config: {e}")).into_response();
        }
        copied.push("config.toml");
    }
    if db.exists() {
        if let Err(e) = std::fs::copy(&db, dest.join("jobs.db")) {
            return crate::problem::Problem::internal(format!("copy jobs.db: {e}")).into_response();
        }
        copied.push("jobs.db");
    }
    (
        StatusCode::CREATED,
        Json(json!({ "name": name, "files": copied, "bytes": walk_dir_bytes(&dest).unwrap_or(0) })),
    )
        .into_response()
}

async fn backups_list(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let dir = backups_dir();
    let mut sets: Vec<serde_json::Value> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            json!({
                "name": name,
                "bytes": walk_dir_bytes(&p).unwrap_or(0),
                "files": std::fs::read_dir(&p)
                    .into_iter().flatten().flatten()
                    .filter_map(|f| f.file_name().to_str().map(String::from))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    // Newest first (names are sortable timestamps).
    sets.sort_by(|a, b| b["name"].as_str().cmp(&a["name"].as_str()));
    Json(json!({ "backups": sets })).into_response()
}

async fn backup_restore(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    if !safe_backup_name(&name) {
        return crate::problem::Problem::bad_request("invalid backup name").into_response();
    }
    let src = backups_dir().join(&name);
    if !src.is_dir() {
        return crate::problem::Problem::not_found("backup not found").into_response();
    }
    let mut restored = Vec::new();
    let cfg_src = src.join("config.toml");
    if cfg_src.exists() {
        if let Err(e) = std::fs::copy(&cfg_src, strivo_core::config::AppConfig::config_path()) {
            return crate::problem::Problem::internal(format!("restore config: {e}")).into_response();
        }
        restored.push("config.toml");
    }
    let db_src = src.join("jobs.db");
    if db_src.exists() {
        let db_dest = strivo_core::config::AppConfig::data_dir().join("jobs.db");
        if let Err(e) = std::fs::copy(&db_src, &db_dest) {
            return crate::problem::Problem::internal(format!("restore jobs.db: {e}")).into_response();
        }
        restored.push("jobs.db");
    }
    Json(json!({
        "restored": restored,
        "note": "Restart the daemon for the restored config/DB to take effect.",
    }))
    .into_response()
}

/// `GET /api/v1/history` — durable recording history from the jobs DB
/// (roadmap item 17). Unlike `/recordings` (the in-memory, bounded daemon
/// snapshot), this survives restarts and includes completed/failed jobs.
async fn history(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let db = match open_jobs_db() {
        Ok(d) => d,
        Err(e) => return crate::problem::Problem::internal(e).into_response(),
    };
    match db.load_recording_jobs().await {
        Ok(mut jobs) => {
            jobs.sort_by_key(|j| std::cmp::Reverse(j.started_at));
            Json(json!({ "history": jobs })).into_response()
        }
        Err(e) => crate::problem::Problem::internal(e.to_string()).into_response(),
    }
}

// ── Blocklist (roadmap item 17) ──────────────────────────────────────

fn parse_platform(s: &str) -> Option<PlatformKind> {
    match s.to_ascii_lowercase().as_str() {
        "twitch" => Some(PlatformKind::Twitch),
        "youtube" => Some(PlatformKind::YouTube),
        "patreon" => Some(PlatformKind::Patreon),
        _ => None,
    }
}

fn open_jobs_db() -> Result<strivo_core::recording::persist::PersistDb, String> {
    let db_path = strivo_core::config::AppConfig::data_dir().join("jobs.db");
    strivo_core::recording::persist::PersistDb::open(&db_path).map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
struct BlockPayload {
    platform: String,
    channel_id: String,
    #[serde(default)]
    vod_id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

async fn blocklist_get(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let db = match open_jobs_db() {
        Ok(d) => d,
        Err(e) => return crate::problem::Problem::internal(e).into_response(),
    };
    match db.list_blocklist().await {
        Ok(entries) => Json(json!({ "blocklist": entries })).into_response(),
        Err(e) => crate::problem::Problem::internal(e.to_string()).into_response(),
    }
}

async fn blocklist_add(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<BlockPayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let Some(platform) = parse_platform(&body.platform) else {
        return crate::problem::Problem::bad_request("unknown platform").into_response();
    };
    let db = match open_jobs_db() {
        Ok(d) => d,
        Err(e) => return crate::problem::Problem::internal(e).into_response(),
    };
    match db
        .add_blocklist(platform, &body.channel_id, body.vod_id.as_deref(), body.reason.as_deref())
        .await
    {
        Ok(()) => (StatusCode::CREATED, Json(json!({"status": "blocked"}))).into_response(),
        Err(e) => crate::problem::Problem::internal(e.to_string()).into_response(),
    }
}

async fn blocklist_remove(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<BlockPayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let Some(platform) = parse_platform(&body.platform) else {
        return crate::problem::Problem::bad_request("unknown platform").into_response();
    };
    let db = match open_jobs_db() {
        Ok(d) => d,
        Err(e) => return crate::problem::Problem::internal(e).into_response(),
    };
    match db
        .remove_blocklist(platform, &body.channel_id, body.vod_id.as_deref())
        .await
    {
        Ok(()) => Json(json!({"status": "unblocked"})).into_response(),
        Err(e) => crate::problem::Problem::internal(e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct AutoRecordPayload {
    enabled: bool,
}

/// `PUT /api/v1/channels/<channel_key>/auto_record` — toggle
/// `auto_record_channels` membership for the given Platform:id key. (W1.)
async fn put_auto_record(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(channel_key): Path<String>,
    Json(body): Json<AutoRecordPayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let mut cfg = match strivo_core::config::AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => {
            return crate::problem::Problem::internal(e.to_string()).into_response();
        }
    };
    let already_in = cfg
        .auto_record_channels
        .iter()
        .any(|c| format!("{}:{}", c.platform, c.channel_id) == channel_key);
    match (body.enabled, already_in) {
        (true, false) => {
            let Some((plat_str, ch_id)) = channel_key.split_once(':') else {
                return crate::problem::Problem::bad_request("channel_key must be Platform:id")
                    .into_response();
            };
            let platform = match plat_str.to_lowercase().as_str() {
                "twitch" => PlatformKind::Twitch,
                "youtube" => PlatformKind::YouTube,
                "patreon" => PlatformKind::Patreon,
                _ => {
                    return crate::problem::Problem::bad_request(format!(
                        "unknown platform: {plat_str}"
                    ))
                    .into_response();
                }
            };
            // Look up the channel's display name from the snapshot —
            // AutoRecordEntry requires it. Fall back to the platform
            // identifier when the channel isn't currently in the
            // cached snapshot (rare; only happens before the first
            // monitor poll completes).
            let display_name = match state.ipc.snapshot().await {
                Ok(ServerMessage::StateSnapshot { channels, .. }) => channels
                    .iter()
                    .find(|c| c.id == ch_id)
                    .map(|c| c.display_name.clone())
                    .unwrap_or_else(|| ch_id.to_string()),
                _ => ch_id.to_string(),
            };
            cfg.auto_record_channels
                .push(strivo_core::config::AutoRecordEntry {
                    platform: format!("{platform:?}"),
                    channel_id: ch_id.to_string(),
                    channel_name: display_name,
                    format: None,
                    profile: None,
                });
        }
        (false, true) => {
            cfg.auto_record_channels
                .retain(|c| format!("{}:{}", c.platform, c.channel_id) != channel_key);
        }
        _ => {}
    }
    if let Err(e) = cfg.save(None) {
        return crate::problem::Problem::internal(e.to_string()).into_response();
    }
    let _ = state.ipc.send_command(ClientMessage::PollNow).await;
    (
        StatusCode::OK,
        Json(json!({"status": "ok", "enabled": body.enabled})),
    )
        .into_response()
}

// ── W2: plugin RPC ───────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
struct PluginRpcPayload {
    #[serde(default)]
    selection: Vec<Uuid>,
    #[serde(default)]
    payload: serde_json::Value,
}

/// `POST /api/v1/plugins/<plugin>/<verb>` — dispatch an actions-popup
/// verb to a plugin. Body is `{ selection: [uuid…], payload: any }`.
/// In daemon mode the plugin registry isn't loaded inside the daemon
/// process yet (W2 phase 2 follow-up); today the call lands in the
/// daemon's log so the webui can surface the "not loaded" affordance.
async fn plugin_rpc(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path((plugin, verb)): Path<(String, String)>,
    body: Option<Json<PluginRpcPayload>>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let Json(body) = body.unwrap_or(Json(PluginRpcPayload::default()));
    let cmd = ClientMessage::PluginRpc {
        plugin: plugin.clone(),
        verb: verb.clone(),
        selection: body.selection,
        payload: body.payload,
    };
    match state.ipc.send_command(cmd).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "status": "queued",
                "plugin": plugin,
                "verb": verb,
                "note": "dispatched in the daemon plugin host (W2-phase-3); SpawnTask work runs headless"
            })),
        )
            .into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct BulkDownloadPayload {
    channel_name: String,
    platform: PlatformKind,
    /// "start" | "stop"
    action: String,
    #[serde(default)]
    playlist_id: Option<String>,
}

/// `POST /api/v1/channels/{id}/bulk` — start or stop a per-channel bulk
/// back-catalog download. Mirrors the TUI's `b` toggle (#71) and the
/// playlist-scoped Shift+P picker (#73). Progress streams back over
/// `/events` as `bulk-progress`. (W#74.)
async fn bulk_download(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(channel_id): Path<String>,
    Json(body): Json<BulkDownloadPayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let action = match body.action.as_str() {
        "start" => BulkAction::Start,
        "stop" => BulkAction::Stop,
        other => {
            return crate::problem::Problem::bad_request(format!("unknown action {other:?}"))
                .into_response()
        }
    };
    let cmd = ClientMessage::BulkDownload {
        channel_id,
        channel_name: body.channel_name,
        platform: body.platform,
        action,
        playlist_id: body.playlist_id,
    };
    match state.ipc.send_command(cmd).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "queued"}))).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

/// `POST /api/v1/channels/{id}/playlists` — request the channel's
/// YouTube playlists for the scope picker. The list arrives over
/// `/events` as `playlist-list`. (W#74 / #73.)
async fn request_playlists(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(channel_id): Path<String>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state
        .ipc
        .send_command(ClientMessage::ListPlaylists { channel_id })
        .await
    {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({"status": "requested", "note": "result arrives via /events playlist-list"})),
        )
            .into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct ChannelVodsPayload {
    platform: PlatformKind,
}

#[derive(Debug, Deserialize)]
struct ResolvePayload {
    platform: PlatformKind,
    query: String,
}

/// `POST /api/v1/channels/resolve` — resolve a human identifier (Twitch
/// login, YouTube/Patreon id) for the Add-Channel wizard (task #19). The
/// result arrives over `/events` as a `ChannelResolved` frame.
async fn resolve_channel(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<ResolvePayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state
        .ipc
        .send_command(ClientMessage::ResolveChannel {
            platform: body.platform,
            query: body.query,
        })
        .await
    {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({"status": "requested", "note": "result arrives via /events ChannelResolved"})),
        )
            .into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

/// `POST /api/v1/channels/{id}/vods` — request a channel's recent VODs
/// (live broadcasts + uploads) for the detail pane. The list arrives over
/// `/events` as `channel-vods`. (TUI-style redesign.)
async fn request_channel_vods(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(channel_id): Path<String>,
    Json(body): Json<ChannelVodsPayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    match state
        .ipc
        .send_command(ClientMessage::FetchChannelVods {
            channel_id,
            platform: body.platform,
        })
        .await
    {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({"status": "requested", "note": "result arrives via /events channel-vods"})),
        )
            .into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct PatreonPullPayload {
    embed_url: String,
    creator_name: String,
    post_title: String,
}

/// `POST /api/v1/patreon/pull` — download a single Patreon video post on
/// demand. Webui equivalent of the TUI's `p` on a creator's post (#69).
/// The daemon builds the output path from its config. (#75.)
async fn patreon_pull(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<PatreonPullPayload>,
) -> impl IntoResponse {
    if check_key(&headers, &state).is_err() {
        return crate::problem::Problem::unauthorized().into_response();
    }
    let cmd = ClientMessage::PatreonPull {
        embed_url: body.embed_url,
        creator_name: body.creator_name,
        post_title: body.post_title,
    };
    match state.ipc.send_command(cmd).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "queued"}))).into_response(),
        Err(e) => crate::problem::Problem::unavailable(e.to_string()).into_response(),
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/health/checks", get(health_checks))
        .route("/api/v1/channels", get(channels))
        .route("/api/v1/patreon", get(patreon))
        .route("/api/v1/recordings", get(recordings).post(start_recording))
        .route(
            "/api/v1/recordings/{id}",
            get(recording_one).delete(stop_recording),
        )
        .route("/api/v1/recordings/stop_all", post(stop_all_recordings))
        .route("/api/v1/schedule", get(schedule))
        .route("/api/v1/settings", get(settings))
        .route("/api/v1/poll_now", post(poll_now))
        .route("/api/v1/settings/poll_interval", post(set_poll_interval))
        .route("/api/v1/logs", get(logs))
        .route("/api/v1/history", get(history))
        .route("/api/v1/backup", post(backup_create))
        .route("/api/v1/backups", get(backups_list))
        .route("/api/v1/backups/{name}/restore", post(backup_restore))
        .route(
            "/api/v1/blocklist",
            get(blocklist_get).post(blocklist_add).delete(blocklist_remove),
        )
        .route(
            "/api/v1/channels/{channel_key}/auto_record",
            put(put_auto_record),
        )
        .route("/api/v1/plugins/{plugin}/{verb}", post(plugin_rpc))
        // W5: stream-recorder surfaces
        .route("/api/v1/storage", get(storage))
        .route("/api/v1/gantt", get(gantt))
        // #74: bulk-download controls
        .route("/api/v1/channels/{channel_id}/bulk", post(bulk_download))
        .route(
            "/api/v1/channels/{channel_id}/playlists",
            post(request_playlists),
        )
        .route(
            "/api/v1/channels/{channel_id}/vods",
            post(request_channel_vods),
        )
        // #75: Patreon manual pull
        .route("/api/v1/patreon/pull", post(patreon_pull))
        // #19: Add-Channel wizard — resolve a name/id to a channel.
        .route("/api/v1/channels/resolve", post(resolve_channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_name_rejects_traversal() {
        assert!(safe_backup_name("2026-05-26T22-30-00Z"));
        assert!(safe_backup_name("snapshot_1.bak"));
        assert!(!safe_backup_name(""));
        assert!(!safe_backup_name(".."));
        assert!(!safe_backup_name("../etc"));
        assert!(!safe_backup_name("a/b"));
        assert!(!safe_backup_name("with space"));
    }

    #[test]
    fn log_level_parse_and_rank() {
        assert_eq!(line_level("2026-05-26T00:00:00Z  INFO mod: hi"), Some("INFO"));
        assert_eq!(line_level("2026-05-26T00:00:00Z ERROR mod: boom"), Some("ERROR"));
        assert_eq!(line_level("continuation line with no level"), None);
        assert!(level_rank("ERROR") > level_rank("WARN"));
        assert!(level_rank("WARN") > level_rank("INFO"));
        assert!(level_rank("INFO") > level_rank("DEBUG"));
        assert!(level_rank("DEBUG") > level_rank("TRACE"));
    }

    #[test]
    fn disk_severity_thresholds() {
        assert_eq!(disk_severity(0, 0), "warn"); // unknown
        assert_eq!(disk_severity(100, 2), "error"); // 2% free
        assert_eq!(disk_severity(100, 10), "warn"); // 10% free
        assert_eq!(disk_severity(100, 50), "ok"); // 50% free
        assert_eq!(disk_severity(100, 15), "ok"); // exactly 15% → ok (>= boundary)
        assert_eq!(disk_severity(100, 14), "warn");
    }
}
