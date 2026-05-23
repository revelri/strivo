//! /settings full surface (webui phase 7) — mirrors the TUI's M2 group
//! layout. Each row is one of: bool toggle, free-text edit, plain
//! status string. Secrets (twitch.client_secret, etc.) are never
//! rendered.

use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use strivo_core::config::AppConfig;

use crate::server::AppState;

#[derive(Debug, Clone)]
pub struct SettingRow {
    pub key: String,
    pub label: String,
    pub kind: String, // "bool" | "string" | "status"
    pub value: String,
    pub bool_val: bool,
    pub hint: String,
}

#[derive(Debug, Clone)]
pub struct SettingGroup {
    pub title: String,
    pub rows: Vec<SettingRow>,
}

#[derive(Template)]
#[template(path = "settings.html")]
struct PageTemplate {
    title: &'static str,
    groups: Vec<SettingGroup>,
}

fn build_groups(cfg: &AppConfig) -> Vec<SettingGroup> {
    let mut groups = Vec::new();

    let recording = SettingGroup {
        title: "Recording".into(),
        rows: vec![
            SettingRow {
                key: "recording_dir".into(),
                label: "Output directory".into(),
                kind: "string".into(),
                value: cfg.recording_dir.to_string_lossy().into_owned(),
                bool_val: false,
                hint: "~ expands".into(),
            },
            SettingRow {
                key: "filename_template".into(),
                label: "Filename template".into(),
                kind: "string".into(),
                value: cfg.recording.filename_template.clone(),
                bool_val: false,
                hint: "{channel} {date} {title}".into(),
            },
            SettingRow {
                key: "transcode".into(),
                label: "Transcode".into(),
                kind: "bool".into(),
                value: String::new(),
                bool_val: cfg.recording.transcode,
                hint: "NVENC passthrough".into(),
            },
        ],
    };
    groups.push(recording);

    let archiver = SettingGroup {
        title: "Archiver".into(),
        rows: vec![
            SettingRow {
                key: "archiver.enabled".into(),
                label: "Enabled".into(),
                kind: "bool".into(),
                value: String::new(),
                bool_val: cfg.archiver.enabled,
                hint: "back-catalog scanner".into(),
            },
            SettingRow {
                key: "archiver.archive_dir".into(),
                label: "Archive directory".into(),
                kind: "string".into(),
                value: cfg.archiver.archive_dir.to_string_lossy().into_owned(),
                bool_val: false,
                hint: String::new(),
            },
            SettingRow {
                key: "archiver.format".into(),
                label: "yt-dlp format".into(),
                kind: "string".into(),
                value: cfg.archiver.format.clone(),
                bool_val: false,
                hint: String::new(),
            },
        ],
    };
    groups.push(archiver);

    let crunchr = SettingGroup {
        title: "Crunchr".into(),
        rows: vec![
            SettingRow {
                key: "crunchr.enabled".into(),
                label: "Enabled".into(),
                kind: "bool".into(),
                value: String::new(),
                bool_val: cfg.crunchr.enabled,
                hint: "transcription + analysis".into(),
            },
            SettingRow {
                key: "crunchr.whisper_model".into(),
                label: "Whisper model".into(),
                kind: "string".into(),
                value: cfg.crunchr.whisper_model.clone().unwrap_or_default(),
                bool_val: false,
                hint: "auto if blank".into(),
            },
            SettingRow {
                key: "crunchr.analysis.enabled".into(),
                label: "Analysis enabled".into(),
                kind: "bool".into(),
                value: String::new(),
                bool_val: cfg.crunchr.analysis.enabled,
                hint: "topic + summary".into(),
            },
            SettingRow {
                key: "crunchr.analysis.model".into(),
                label: "Analysis model".into(),
                kind: "string".into(),
                value: cfg.crunchr.analysis.model.clone(),
                bool_val: false,
                hint: "OpenRouter slug".into(),
            },
        ],
    };
    groups.push(crunchr);

    let output = SettingGroup {
        title: "Output".into(),
        rows: vec![
            SettingRow {
                key: "poll_interval_secs".into(),
                label: "Poll interval (s)".into(),
                kind: "string".into(),
                value: cfg.poll_interval_secs.to_string(),
                bool_val: false,
                hint: "≥15".into(),
            },
            SettingRow {
                key: "ui.reduce_motion".into(),
                label: "Reduce motion".into(),
                kind: "bool".into(),
                value: String::new(),
                bool_val: cfg.ui.reduce_motion,
                hint: "snap animations".into(),
            },
            SettingRow {
                key: "ui.verbose_status".into(),
                label: "Verbose status".into(),
                kind: "bool".into(),
                value: String::new(),
                bool_val: cfg.ui.verbose_status,
                hint: "long labels".into(),
            },
        ],
    };
    groups.push(output);

    let connections = SettingGroup {
        title: "Connections".into(),
        rows: vec![
            SettingRow {
                key: "twitch".into(),
                label: "Twitch".into(),
                kind: "status".into(),
                value: status_label(cfg.twitch.is_some()),
                bool_val: false,
                hint: "wizard required to add".into(),
            },
            SettingRow {
                key: "youtube".into(),
                label: "YouTube".into(),
                kind: "status".into(),
                value: status_label(cfg.youtube.is_some()),
                bool_val: false,
                hint: "wizard required to add".into(),
            },
            SettingRow {
                key: "patreon".into(),
                label: "Patreon".into(),
                kind: "status".into(),
                value: status_label(cfg.patreon.is_some()),
                bool_val: false,
                hint: "wizard required to add".into(),
            },
        ],
    };
    groups.push(connections);

    groups
}

fn status_label(configured: bool) -> String {
    if configured {
        "configured".into()
    } else {
        "not configured".into()
    }
}

async fn page(State(_state): State<AppState>) -> Response {
    let cfg = match AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return Html(format!("<h1>load failed</h1><pre>{e}</pre>")).into_response(),
    };
    render(
        PageTemplate {
            title: "Settings",
            groups: build_groups(&cfg),
        }
        .render(),
    )
}

#[derive(Deserialize)]
struct UpdateForm {
    value: String,
}

async fn update(
    State(_state): State<AppState>,
    Path(key): Path<String>,
    Form(form): Form<UpdateForm>,
) -> Response {
    let mut cfg = match AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return Html(format!("save: load failed: {e}")).into_response(),
    };
    let value = form.value;
    let ok = apply(&mut cfg, &key, &value);
    if !ok {
        return Html(format!("unknown setting key: {key}")).into_response();
    }
    if let Err(e) = cfg.save(None) {
        return Html(format!("save: {e}")).into_response();
    }
    Redirect::to("/settings").into_response()
}

fn apply(cfg: &mut AppConfig, key: &str, value: &str) -> bool {
    let truthy = value == "true" || value == "on" || value == "1";
    match key {
        "recording_dir" => cfg.recording_dir = std::path::PathBuf::from(value),
        "filename_template" => cfg.recording.filename_template = value.into(),
        "transcode" => cfg.recording.transcode = truthy,
        "archiver.enabled" => cfg.archiver.enabled = truthy,
        "archiver.archive_dir" => cfg.archiver.archive_dir = std::path::PathBuf::from(value),
        "archiver.format" => cfg.archiver.format = value.into(),
        "crunchr.enabled" => cfg.crunchr.enabled = truthy,
        "crunchr.whisper_model" => {
            cfg.crunchr.whisper_model = (!value.is_empty()).then(|| value.into());
        }
        "crunchr.analysis.enabled" => cfg.crunchr.analysis.enabled = truthy,
        "crunchr.analysis.model" => cfg.crunchr.analysis.model = value.into(),
        "poll_interval_secs" => {
            if let Ok(n) = value.parse::<u64>() {
                cfg.poll_interval_secs = n.max(15);
            }
        }
        "ui.reduce_motion" => cfg.ui.reduce_motion = truthy,
        "ui.verbose_status" => cfg.ui.verbose_status = truthy,
        _ => return false,
    }
    true
}

async fn reset(State(_state): State<AppState>) -> Response {
    let mut cfg = match AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return Html(format!("reset: load failed: {e}")).into_response(),
    };
    cfg.reset_to_defaults();
    if let Err(e) = cfg.save(None) {
        return Html(format!("reset: save failed: {e}")).into_response();
    }
    Redirect::to("/settings").into_response()
}

fn render(r: Result<String, askama::Error>) -> Response {
    match r {
        Ok(html) => Html(html).into_response(),
        Err(e) => Html(format!("<pre>{e}</pre>")).into_response(),
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings", get(page))
        .route("/settings/reset", post(reset))
        .route("/settings/{key}", post(update))
}
