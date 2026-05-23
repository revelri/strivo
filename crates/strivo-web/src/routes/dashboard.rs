//! Dashboard route + the partial fragments htmx swaps in on SSE
//! events (phase 3).
//!
//! The full page renders the three sections (cards, live channels,
//! active recordings) by inlining the same partials a follow-up GET
//! will fetch. Each partial is its own route under `/_partials/` so
//! htmx can target them with `hx-get` + `hx-trigger="sse:… from:body"`.

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use strivo_core::ipc::ServerMessage;

use crate::server::AppState;

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    title: &'static str,
    channel_count: usize,
    live_count: usize,
    recording_count: usize,
    twitch_connected: bool,
    youtube_connected: bool,
    patreon_connected: bool,
    live_channels: Vec<LiveRow>,
    active_recordings: Vec<RecordingRow>,
}

#[derive(Template)]
#[template(path = "_dashboard_cards.html")]
struct CardsPartial {
    channel_count: usize,
    live_count: usize,
    recording_count: usize,
    twitch_connected: bool,
    youtube_connected: bool,
    patreon_connected: bool,
}

#[derive(Template)]
#[template(path = "_dashboard_live.html")]
struct LivePartial {
    live_channels: Vec<LiveRow>,
}

#[derive(Template)]
#[template(path = "_dashboard_active.html")]
struct ActivePartial {
    active_recordings: Vec<RecordingRow>,
}

#[derive(Debug, Clone)]
pub struct LiveRow {
    pub platform: String,
    pub display_name: String,
    pub title: String,
    pub viewer_count: u64,
}

#[derive(Debug, Clone)]
pub struct RecordingRow {
    pub state: String,
    pub channel_name: String,
    pub bytes_human: String,
    pub duration_human: String,
}

async fn build_view(state: &AppState) -> Result<DashboardTemplate, String> {
    let snap = state.ipc.snapshot().await.map_err(|e| e.to_string())?;
    let ServerMessage::StateSnapshot {
        channels,
        recordings,
        twitch_connected,
        youtube_connected,
        patreon_connected,
        ..
    } = snap
    else {
        return Err("unexpected ServerMessage".into());
    };

    let live_channels: Vec<LiveRow> = channels
        .iter()
        .filter(|c| c.is_live)
        .map(|c| LiveRow {
            platform: c.platform.to_string(),
            display_name: c.display_name.clone(),
            title: c.stream_title.clone().unwrap_or_default(),
            viewer_count: c.viewer_count.unwrap_or(0),
        })
        .collect();

    let active_recordings: Vec<RecordingRow> = recordings
        .values()
        .filter(|j| {
            use strivo_core::recording::job::RecordingState as S;
            matches!(j.state, S::Recording | S::ResolvingUrl | S::Stopping)
        })
        .map(|j| RecordingRow {
            state: format!("{:?}", j.state).to_lowercase(),
            channel_name: j.channel_name.clone(),
            bytes_human: human_bytes(j.bytes_written),
            duration_human: format_duration(j.duration_secs),
        })
        .collect();

    Ok(DashboardTemplate {
        title: "Dashboard",
        channel_count: channels.len(),
        live_count: live_channels.len(),
        recording_count: active_recordings.len(),
        twitch_connected,
        youtube_connected,
        patreon_connected,
        live_channels,
        active_recordings,
    })
}

async fn dashboard(State(state): State<AppState>) -> Response {
    match build_view(&state).await {
        Ok(v) => render_or_err(v.render()),
        Err(e) => Html(format!("<h1>daemon unreachable</h1><pre>{e}</pre>")).into_response(),
    }
}

async fn cards(State(state): State<AppState>) -> Response {
    match build_view(&state).await {
        Ok(v) => render_or_err(
            CardsPartial {
                channel_count: v.channel_count,
                live_count: v.live_count,
                recording_count: v.recording_count,
                twitch_connected: v.twitch_connected,
                youtube_connected: v.youtube_connected,
                patreon_connected: v.patreon_connected,
            }
            .render(),
        ),
        Err(e) => Html(format!("<div class=err>{e}</div>")).into_response(),
    }
}

async fn live(State(state): State<AppState>) -> Response {
    match build_view(&state).await {
        Ok(v) => render_or_err(LivePartial { live_channels: v.live_channels }.render()),
        Err(e) => Html(format!("<li class=err>{e}</li>")).into_response(),
    }
}

async fn active(State(state): State<AppState>) -> Response {
    match build_view(&state).await {
        Ok(v) => render_or_err(
            ActivePartial {
                active_recordings: v.active_recordings,
            }
            .render(),
        ),
        Err(e) => Html(format!("<li class=err>{e}</li>")).into_response(),
    }
}

fn render_or_err(r: Result<String, askama::Error>) -> Response {
    match r {
        Ok(html) => Html(html).into_response(),
        Err(e) => Html(format!("<pre>render error: {e}</pre>")).into_response(),
    }
}

fn human_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if b >= GB {
        format!("{:.2} GB", b as f64 / GB as f64)
    } else if b >= MB {
        format!("{:.1} MB", b as f64 / MB as f64)
    } else if b >= KB {
        format!("{:.0} KB", b as f64 / KB as f64)
    } else {
        format!("{b} B")
    }
}

fn format_duration(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/_partials/dashboard-cards", get(cards))
        .route("/_partials/dashboard-live", get(live))
        .route("/_partials/dashboard-active", get(active))
}
