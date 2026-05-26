//! SSE relay of daemon events to the browser (webui phase 2).
//!
//! `GET /events` opens a persistent connection on the IPC socket via
//! [`IpcClient::events`] and emits one Server-Sent Event per
//! `DaemonEvent`. HTMX `hx-sse="connect:/events"` subscribers see
//! every channel-went-live, recording-progress, schedule-fired, etc.
//! as it happens.
//!
//! The body is a single `event: <variant>\ndata: <json>\n\n` per
//! daemon event. Clients filter by event name using `hx-sse` selectors
//! or fall back to a plain `data:` listener.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use futures::stream::{Stream, StreamExt};

use crate::server::AppState;

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut event_stream = state.ipc.events();
    let stream = async_stream::stream! {
        while let Some(item) = event_stream.next().await {
            match item {
                Ok(de) => {
                    let variant = daemon_event_kind(&de);
                    match serde_json::to_string(&de) {
                        Ok(body) => {
                            yield Ok(Event::default().event(variant).data(body));
                        }
                        Err(e) => {
                            tracing::warn!("event JSON encode failed: {e}");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("daemon event stream error: {e}");
                    yield Ok(Event::default()
                        .event("error")
                        .data(e.to_string()));
                    break;
                }
            }
        }
    };
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

/// Stable string label for the SSE `event:` field — keeps the wire
/// format independent of the Rust enum discriminant order.
fn daemon_event_kind(de: &strivo_core::app::DaemonEvent) -> &'static str {
    use strivo_core::app::DaemonEvent as D;
    match de {
        D::ChannelsUpdated(_) => "channels-updated",
        D::ChannelWentLive(_) => "channel-live",
        D::ChannelWentOffline(_) => "channel-offline",
        D::StreamUrlResolved { .. } => "stream-url-resolved",
        D::RecordingStarted { .. } => "recording-started",
        D::RecordingProgress { .. } => "recording-progress",
        D::RecordingFinished { .. } => "recording-finished",
        D::Notification { .. } => "notification",
        D::AllRecordingsStopped => "all-stopped",
        D::DeviceCodeRequired { .. } => "device-code-required",
        D::PlatformAuthenticated { .. } => "platform-authenticated",
        D::PatreonPostFound { .. } => "patreon-post",
        D::PatreonState { .. } => "patreon-state",
        D::BulkProgress { .. } => "bulk-progress",
        D::PlaylistList { .. } => "playlist-list",
        D::ScheduleFired { .. } => "schedule-fired",
        D::Error(_) => "error",
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(events))
}
