//! YouTube WebSub (PubSubHubbub) callback — the public endpoint Google's hub
//! talks to. Mounted at `/yt-websub` and exposed to the internet via
//! `tailscale funnel` so the hub can reach it.
//!
//! - `GET  /yt-websub` — subscription verification. The hub appends
//!   `hub.challenge`; we echo it verbatim (text/plain) to confirm the
//!   subscribe/unsubscribe. Without it there's nothing to verify → 400.
//! - `POST /yt-websub` — a content notification (Atom XML) that one of our
//!   subscribed channels published / went live. We don't trust or parse the
//!   body for control flow; any notification simply fires `PollNow` over IPC,
//!   reusing the monitor's batched live check (RSS + one `videos.list`) to
//!   confirm and, if live, auto-record. Return 204 fast so the hub is happy.
//!
//! These routes are intentionally unauthenticated (the hub sends no API key or
//! CSRF token) and are merged *after* the auth/CSRF layers in `server.rs`.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use strivo_core::ipc::ClientMessage;

use crate::server::AppState;

const PATH: &str = "/yt-websub";

pub fn router() -> Router<AppState> {
    Router::new().route(PATH, get(verify).post(notify))
}

/// Subscription verification handshake: echo `hub.challenge` back to the hub.
async fn verify(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    match params.get("hub.challenge") {
        Some(c) => {
            tracing::info!(
                "youtube websub: verified hub.mode={:?}",
                params.get("hub.mode")
            );
            (StatusCode::OK, c.clone()).into_response()
        }
        None => (StatusCode::BAD_REQUEST, "missing hub.challenge").into_response(),
    }
}

/// Content notification: nudge the daemon to poll now. We don't parse the
/// Atom body — a notification only means "something changed, check soon",
/// and the monitor's poll re-derives live state authoritatively.
async fn notify(State(state): State<AppState>) -> impl IntoResponse {
    match state.ipc.send_command(ClientMessage::PollNow).await {
        Ok(()) => tracing::info!("youtube websub: notification -> PollNow"),
        Err(e) => tracing::warn!("youtube websub: PollNow forward failed: {e:#}"),
    }
    // The hub only cares that we accepted it; 204 with no body is standard.
    StatusCode::NO_CONTENT
}
