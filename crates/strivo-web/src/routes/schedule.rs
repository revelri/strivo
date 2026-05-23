//! /schedule calendar (webui phase 6).
//!
//! Renders config.schedule with next-fire times computed via the same
//! `cron::Schedule` crate the TUI uses. CRUD operations mutate
//! config.toml directly, mirroring channels.rs.

use std::str::FromStr;

use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get};
use axum::Router;
use cron::Schedule;
use serde::Deserialize;
use strivo_core::config::{AppConfig, ScheduleEntry};

use crate::server::AppState;

#[derive(Debug, Clone)]
pub struct ScheduleRow {
    pub index: usize,
    pub channel: String,
    pub cron: String,
    pub duration: String,
    pub next_fire: String,
}

#[derive(Template)]
#[template(path = "schedule.html")]
struct PageTemplate {
    title: &'static str,
    rows: Vec<ScheduleRow>,
}

#[derive(Template)]
#[template(path = "_schedule_list.html")]
struct ListPartial {
    rows: Vec<ScheduleRow>,
}

fn next_fire(cron: &str) -> String {
    let expr = if cron.split_whitespace().count() == 5 {
        format!("0 {cron}")
    } else {
        cron.to_string()
    };
    match Schedule::from_str(&expr) {
        Ok(s) => s
            .upcoming(chrono::Utc)
            .next()
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%a %b %-d · %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "—".to_string()),
        Err(e) => format!("cron error: {e}"),
    }
}

fn rows_from(cfg: &AppConfig) -> Vec<ScheduleRow> {
    cfg.schedule
        .iter()
        .enumerate()
        .map(|(i, s)| ScheduleRow {
            index: i,
            channel: s.channel.clone(),
            cron: s.cron.clone(),
            duration: s.duration.clone(),
            next_fire: next_fire(&s.cron),
        })
        .collect()
}

async fn page(State(_state): State<AppState>) -> Response {
    let cfg = match AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return Html(format!("<h1>load config failed</h1><pre>{e}</pre>")).into_response(),
    };
    render(
        PageTemplate {
            title: "Calendar",
            rows: rows_from(&cfg),
        }
        .render(),
    )
}

#[derive(Deserialize)]
struct AddForm {
    channel: String,
    cron: String,
    duration: String,
}

async fn add(State(_state): State<AppState>, Form(form): Form<AddForm>) -> Response {
    let mut cfg = match AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return list_err(&format!("load config: {e}")),
    };
    if let Err(e) = validate_cron(&form.cron) {
        return list_err(&format!("invalid cron: {e}"));
    }
    cfg.schedule.push(ScheduleEntry {
        channel: form.channel,
        cron: form.cron,
        duration: form.duration,
    });
    if let Err(e) = cfg.save(None) {
        return list_err(&format!("save: {e}"));
    }
    render(ListPartial { rows: rows_from(&cfg) }.render())
}

async fn remove(State(_state): State<AppState>, Path(index): Path<usize>) -> Response {
    let mut cfg = match AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return list_err(&format!("load config: {e}")),
    };
    if index < cfg.schedule.len() {
        cfg.schedule.remove(index);
    }
    if let Err(e) = cfg.save(None) {
        return list_err(&format!("save: {e}"));
    }
    render(ListPartial { rows: rows_from(&cfg) }.render())
}

fn validate_cron(cron: &str) -> Result<(), String> {
    let expr = if cron.split_whitespace().count() == 5 {
        format!("0 {cron}")
    } else {
        cron.to_string()
    };
    Schedule::from_str(&expr).map(|_| ()).map_err(|e| e.to_string())
}

fn render(r: Result<String, askama::Error>) -> Response {
    match r {
        Ok(html) => Html(html).into_response(),
        Err(e) => Html(format!("<pre>{e}</pre>")).into_response(),
    }
}

fn list_err(msg: &str) -> Response {
    Html(format!("<ul id=schedule-list><li class=err>{msg}</li></ul>")).into_response()
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/schedule", get(page).post(add))
        .route("/schedule/{index}", delete(remove))
}
