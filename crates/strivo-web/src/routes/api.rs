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
use strivo_core::ipc::{ClientMessage, ServerMessage};
use strivo_core::platform::PlatformKind;
use strivo_core::recording::RecordingCommand;
use uuid::Uuid;

use crate::server::AppState;

const API_KEY_HEADER: &str = "x-api-key";

fn check_key(headers: &HeaderMap, state: &AppState) -> Result<(), StatusCode> {
    let key = headers
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if state.api_key.matches(key) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
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
        Ok(cfg) => Json(json!({ "schedule": cfg.schedule })).into_response(),
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
}
