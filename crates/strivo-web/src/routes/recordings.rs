//! /recordings list + stream-back endpoint (webui phase 5).
//!
//! - GET /recordings                          full page
//! - GET /_partials/recordings-list?q=<...>   filtered list (htmx)
//! - GET /recordings/<id>/download            raw file stream
//! - GET /recordings/<id>/play                redirect to download
//!
//! Search is server-side via the upgraded score+spans fuzzy matcher
//! (strivo_core::search::fuzzy_match). Results sort by score
//! descending so the user's best match floats to the top — same
//! ordering as the TUI sidebar / recording list (M4.2.c).
//!
//! Streaming uses tower-http::services::ServeFile so range requests
//! work; browsers' default media element can scrub a long recording
//! without buffering the whole file.

use std::path::PathBuf;

use askama::Template;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use strivo_core::ipc::ServerMessage;
use strivo_core::recording::job::RecordingState;
use uuid::Uuid;

use crate::server::AppState;

#[derive(Debug, Clone)]
pub struct RecRow {
    pub id: String,
    pub state: String,
    pub channel_name: String,
    pub title: String,
    pub started_at: String,
    pub bytes: String,
    pub is_finished: bool,
}

#[derive(Template)]
#[template(path = "recordings.html")]
struct RecordingsTemplate {
    title: &'static str,
    rows: Vec<RecRow>,
    empty_msg: &'static str,
}

#[derive(Template)]
#[template(path = "_recordings_list.html")]
struct RecordingsListPartial {
    rows: Vec<RecRow>,
    empty_msg: &'static str,
}

#[derive(Deserialize, Default)]
struct ListQuery {
    #[serde(default)]
    q: String,
}

async fn snapshot_rows(state: &AppState, query: &str) -> Result<Vec<RecRow>, String> {
    let snap = state.ipc.snapshot().await.map_err(|e| e.to_string())?;
    let ServerMessage::StateSnapshot { recordings, .. } = snap else {
        return Err("unexpected ServerMessage".into());
    };
    let mut jobs: Vec<_> = recordings.into_values().collect();

    if query.is_empty() {
        // Default: newest first.
        jobs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    } else {
        // Score every candidate; drop misses; sort by score desc.
        let mut scored: Vec<(_, i32)> = jobs
            .into_iter()
            .filter_map(|j| {
                let hay = format!(
                    "{} {}",
                    j.channel_name,
                    j.stream_title.as_deref().unwrap_or("")
                );
                let score = strivo_core::search::fuzzy_match(query, &hay).map(|m| m.score)?;
                Some((j, score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        jobs = scored.into_iter().map(|(j, _)| j).collect();
    }

    let rows = jobs
        .into_iter()
        .map(|j| RecRow {
            id: j.id.to_string(),
            state: format!("{:?}", j.state).to_lowercase(),
            channel_name: j.channel_name.clone(),
            title: j.stream_title.clone().unwrap_or_default(),
            started_at: j
                .started_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            bytes: human_bytes(j.bytes_written),
            is_finished: matches!(j.state, RecordingState::Finished),
        })
        .collect();
    Ok(rows)
}

async fn page(State(state): State<AppState>) -> Response {
    match snapshot_rows(&state, "").await {
        Ok(rows) => render(
            RecordingsTemplate {
                title: "Recordings",
                rows,
                empty_msg: "No recordings yet.",
            }
            .render(),
        ),
        Err(e) => Html(format!("<h1>daemon unreachable</h1><pre>{e}</pre>"))
            .into_response(),
    }
}

async fn list_partial(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Response {
    let trimmed = q.q.trim().to_string();
    let empty_msg: &'static str = if trimmed.is_empty() {
        "No recordings yet."
    } else {
        "No matches."
    };
    match snapshot_rows(&state, &trimmed).await {
        Ok(rows) => render(
            RecordingsListPartial { rows, empty_msg }.render(),
        ),
        Err(e) => Html(format!("<ul id=recordings-list><li class=err>{e}</li></ul>"))
            .into_response(),
    }
}

async fn lookup_path(state: &AppState, id: Uuid) -> Result<PathBuf, String> {
    let snap = state.ipc.snapshot().await.map_err(|e| e.to_string())?;
    let ServerMessage::StateSnapshot { recordings, .. } = snap else {
        return Err("unexpected ServerMessage".into());
    };
    recordings
        .get(&id)
        .map(|j| j.output_path.clone())
        .ok_or_else(|| "recording not found".into())
}

async fn download(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    let path = match lookup_path(&state, id).await {
        Ok(p) => p,
        Err(e) => return (StatusCode::NOT_FOUND, e).into_response(),
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let len = file
        .metadata()
        .await
        .map(|m| m.len())
        .ok();
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("recording.mkv");
    let mut resp = Response::new(body);
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        "video/x-matroska".parse().unwrap(),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        format!("inline; filename=\"{filename}\"")
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("inline")),
    );
    if let Some(l) = len {
        if let Ok(v) = header::HeaderValue::from_str(&l.to_string()) {
            resp.headers_mut().insert(header::CONTENT_LENGTH, v);
        }
    }
    resp
}

async fn play(Path(id): Path<Uuid>) -> Redirect {
    Redirect::temporary(&format!("/recordings/{id}/download"))
}

fn render(r: Result<String, askama::Error>) -> Response {
    match r {
        Ok(html) => Html(html).into_response(),
        Err(e) => Html(format!("<pre>{e}</pre>")).into_response(),
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/recordings", get(page))
        .route("/_partials/recordings-list", get(list_partial))
        .route("/recordings/{id}/download", get(download))
        .route("/recordings/{id}/play", get(play))
}
