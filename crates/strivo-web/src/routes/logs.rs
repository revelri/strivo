//! /system/logs (webui phase 8).
//!
//! - GET /system/logs           full page with the last 200 lines.
//! - GET /logs/stream           SSE: one `event: line` per newly-
//!                              appended line. htmx swaps via
//!                              `beforeend`, so the log stays
//!                              chronological with no client JS.
//!
//! Tail via tokio::fs polling — simple, robust, no inotify
//! dependencies. The reader keeps a byte cursor; on each tick it
//! reads from `cursor..len` and emits one event per line.

use std::convert::Infallible;
use std::io::SeekFrom;
use std::time::Duration;

use askama::Template;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures::stream::Stream;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

use crate::server::AppState;

#[derive(Template)]
#[template(path = "logs.html")]
struct LogsTemplate {
    title: &'static str,
    initial: String,
}

fn log_path() -> std::path::PathBuf {
    strivo_core::config::AppConfig::state_dir().join("strivo.log")
}

async fn page(State(_state): State<AppState>) -> Response {
    let path = log_path();
    let body = match tokio::fs::read_to_string(&path).await {
        Ok(s) => {
            // Tail: last 200 lines.
            let lines: Vec<&str> = s.lines().collect();
            let start = lines.len().saturating_sub(200);
            lines[start..].join("\n")
        }
        Err(_) => format!("(log file not found at {})", path.display()),
    };
    let html = LogsTemplate {
        title: "Logs",
        initial: body,
    }
    .render()
    .unwrap_or_else(|e| format!("<pre>render: {e}</pre>"));
    Html(html).into_response()
}

async fn stream(
    State(_state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let path = log_path();
    let stream = async_stream::stream! {
        // Open then seek to end so the SSE only carries *new* lines.
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) => {
                yield Ok(Event::default().event("error").data(format!("{e}")));
                return;
            }
        };
        if let Err(e) = file.seek(SeekFrom::End(0)).await {
            yield Ok(Event::default().event("error").data(format!("seek: {e}")));
            return;
        }
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    // EOF — wait a beat for new bytes.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Ok(_n) => {
                    yield Ok(Event::default().event("line").data(line.trim_end().to_string()));
                }
                Err(e) => {
                    yield Ok(Event::default().event("error").data(format!("{e}")));
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/system/logs", get(page))
        .route("/logs/stream", get(stream))
}
