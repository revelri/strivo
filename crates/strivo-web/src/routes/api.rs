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
use axum::http::{header, HeaderMap, StatusCode};
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

const API_KEY_HEADER: &str = "x-api-key";

/// Authorize a request via EITHER the `X-Api-Key` header (programmatic
/// clients) OR a valid `strivo_session` cookie (browser, set by /login).
/// The browser SPA only carries the cookie, so cookie support is what
/// lets it reach /channels, /recordings, … after logging in. (W3.)
fn check_key(headers: &HeaderMap, state: &AppState) -> Result<(), StatusCode> {
    // 1. X-Api-Key header.
    let key = headers
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !key.is_empty() && state.api_key.matches(key) {
        return Ok(());
    }

    // 2. Signed session cookie.
    if let Some(secret) = state.session_secret.as_deref() {
        if let Some(cookie_header) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
            for pair in cookie_header.split(';') {
                let pair = pair.trim();
                if let Some(val) = pair.strip_prefix(&format!("{}=", crate::routes::login::SESSION_COOKIE)) {
                    if crate::auth::SessionToken::decode_verify(val, secret).is_some() {
                        return Ok(());
                    }
                }
            }
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

async fn channels(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    match state.ipc.snapshot().await {
        Ok(ServerMessage::StateSnapshot { channels, .. }) => {
            Json(json!({ "channels": channels })).into_response()
        }
        Ok(_) => Json(json!({ "channels": [] })).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn recordings(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    match state.ipc.snapshot().await {
        Ok(ServerMessage::StateSnapshot { recordings, .. }) => {
            // recordings is a HashMap<Uuid, RecordingJob>; flatten to a list
            // so the response is stable and order-by-newest-first.
            let mut items: Vec<_> = recordings.into_values().collect();
            items.sort_by(|a, b| b.started_at.cmp(&a.started_at));
            Json(json!({ "recordings": items })).into_response()
        }
        Ok(_) => Json(json!({ "recordings": [] })).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn recording_one(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    match state.ipc.snapshot().await {
        Ok(ServerMessage::StateSnapshot { recordings, .. }) => match recordings.get(&id) {
            Some(j) => Json(j.clone()).into_response(),
            None => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
                .into_response(),
        },
        Ok(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "unexpected response"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn schedule(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn settings(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
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
                "schedule": cfg.schedule,
                "archiver": cfg.archiver,
                "twitch_configured": cfg.twitch.is_some(),
                "youtube_configured": cfg.youtube.is_some(),
                "patreon_configured": cfg.patreon.is_some(),
            });
            Json(body).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))
}

/// `GET /api/v1/storage` — disk usage of the recording directory.
/// (W5 — storage gauges.) Returns bytes_used + bytes_free for the
/// filesystem the recording dir lives on, plus per-platform totals
/// computed by walking the recording-dir tree.
async fn storage(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    let cfg = match strivo_core::config::AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
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
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
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
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
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
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// `DELETE /api/v1/recordings/<id>` — stop the recording with the
/// given job id. Equivalent to the TUI's `s` on a RecordingList row. (W1.)
async fn stop_recording(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    let cmd = ClientMessage::Recording(RecordingCommand::Stop { job_id: id });
    match state.ipc.send_command(cmd).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({"status": "stop sent", "job_id": id})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// `POST /api/v1/recordings/stop_all` — equivalent to the TUI's quit
/// confirmation flow when active recordings are running.
async fn stop_all_recordings(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    match state.ipc.send_command(ClientMessage::Recording(RecordingCommand::StopAll)).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "stop_all sent"}))).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// `POST /api/v1/poll_now` — pokes the channel monitor (TUI sends
/// `ClientMessage::PollNow` via the same path).
async fn poll_now(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    match state.ipc.send_command(ClientMessage::PollNow).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "polled"}))).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
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
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    let mut cfg = match strivo_core::config::AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let already_in = cfg
        .auto_record_channels
        .iter()
        .any(|c| format!("{}:{}", c.platform, c.channel_id) == channel_key);
    match (body.enabled, already_in) {
        (true, false) => {
            let Some((plat_str, ch_id)) = channel_key.split_once(':') else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "channel_key must be Platform:id"})),
                )
                    .into_response();
            };
            let platform = match plat_str.to_lowercase().as_str() {
                "twitch" => PlatformKind::Twitch,
                "youtube" => PlatformKind::YouTube,
                "patreon" => PlatformKind::Patreon,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": format!("unknown platform: {plat_str}")})),
                    )
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
                });
        }
        (false, true) => {
            cfg.auto_record_channels
                .retain(|c| format!("{}:{}", c.platform, c.channel_id) != channel_key);
        }
        _ => {}
    }
    if let Err(e) = cfg.save(None) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response();
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
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
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
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
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
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    let action = match body.action.as_str() {
        "start" => BulkAction::Start,
        "stop" => BulkAction::Stop,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("unknown action {other:?}")})),
            )
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
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
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
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
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
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct ChannelVodsPayload {
    platform: PlatformKind,
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
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
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
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
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
    if let Err(code) = check_key(&headers, &state) {
        return (code, Json(json!({"error": "unauthorized"}))).into_response();
    }
    let cmd = ClientMessage::PatreonPull {
        embed_url: body.embed_url,
        creator_name: body.creator_name,
        post_title: body.post_title,
    };
    match state.ipc.send_command(cmd).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"status": "queued"}))).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/channels", get(channels))
        .route("/api/v1/recordings", get(recordings).post(start_recording))
        .route(
            "/api/v1/recordings/{id}",
            get(recording_one).delete(stop_recording),
        )
        .route("/api/v1/recordings/stop_all", post(stop_all_recordings))
        .route("/api/v1/schedule", get(schedule))
        .route("/api/v1/settings", get(settings))
        .route("/api/v1/poll_now", post(poll_now))
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
}
