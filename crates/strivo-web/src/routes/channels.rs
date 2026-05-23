//! /channels CRUD (webui phase 4).
//!
//! - GET  /channels                                       — full page
//! - POST /channels                                       — add by URL
//! - POST /channels/<platform>/<id>/auto-record           — toggle auto-record
//! - DELETE /channels/<platform>/<id>                     — remove
//!
//! State lives in `config.toml.auto_record_channels`. We mutate the
//! file directly here because the daemon's IPC surface doesn't yet
//! accept config-edit messages — see `webui.9` for that follow-up.
//! Until then, the daemon will pick up changes on its next config
//! reload (poll-driven; not instant but acceptable for an MVP).

use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use serde::Deserialize;
use strivo_core::config::import::candidate_from_url;
use strivo_core::ipc::ServerMessage;

use crate::server::AppState;

#[derive(Debug, Clone)]
pub struct ChannelRow {
    pub id: String,
    pub platform: String,
    pub display_name: String,
    pub is_live: bool,
    pub auto_record: bool,
}

#[derive(Template)]
#[template(path = "channels.html")]
struct ChannelsTemplate {
    title: &'static str,
    channels: Vec<ChannelRow>,
}

#[derive(Template)]
#[template(path = "_channels_list.html")]
struct ChannelsListPartial {
    channels: Vec<ChannelRow>,
}

async fn snapshot_channels(state: &AppState) -> Result<Vec<ChannelRow>, String> {
    let snap = state.ipc.snapshot().await.map_err(|e| e.to_string())?;
    let ServerMessage::StateSnapshot { channels, .. } = snap else {
        return Err("unexpected ServerMessage".into());
    };
    Ok(channels
        .into_iter()
        .map(|c| ChannelRow {
            id: c.id.clone(),
            platform: c.platform.to_string(),
            display_name: c.display_name.clone(),
            is_live: c.is_live,
            auto_record: c.auto_record,
        })
        .collect())
}

async fn page(State(state): State<AppState>) -> Response {
    match snapshot_channels(&state).await {
        Ok(channels) => render(
            ChannelsTemplate {
                title: "Channels",
                channels,
            }
            .render(),
        ),
        Err(e) => Html(format!("<h1>daemon unreachable</h1><pre>{e}</pre>")).into_response(),
    }
}

fn render(r: Result<String, askama::Error>) -> Response {
    match r {
        Ok(html) => Html(html).into_response(),
        Err(e) => Html(format!("<pre>{e}</pre>")).into_response(),
    }
}

async fn list_partial(state: &AppState) -> Response {
    match snapshot_channels(state).await {
        Ok(channels) => render(ChannelsListPartial { channels }.render()),
        Err(e) => Html(format!("<ul id=channels-list><li class=err>{e}</li></ul>"))
            .into_response(),
    }
}

#[derive(Deserialize)]
struct AddForm {
    url: String,
}

async fn add(State(state): State<AppState>, Form(form): Form<AddForm>) -> Response {
    let Some(cand) = candidate_from_url(&form.url, None) else {
        return Html(format!(
            "<ul id=channels-list><li class=err>could not recognize URL: {}</li></ul>",
            form.url
        ))
        .into_response();
    };
    // Load, append, save.
    let mut cfg = match strivo_core::config::AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return Html(format!("<ul id=channels-list><li class=err>load config: {e}</li></ul>")).into_response(),
    };
    let exists = cfg
        .auto_record_channels
        .iter()
        .any(|a| a.platform == cand.platform && a.channel_id == cand.channel_id);
    if !exists {
        cfg.auto_record_channels.push(cand.into_auto_record());
        if let Err(e) = cfg.save(None) {
            return Html(format!("<ul id=channels-list><li class=err>save: {e}</li></ul>")).into_response();
        }
    }
    list_partial(&state).await
}

async fn toggle_auto(
    State(state): State<AppState>,
    Path((platform, id)): Path<(String, String)>,
) -> Response {
    let mut cfg = match strivo_core::config::AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return Html(format!("<span class=err>{e}</span>")).into_response(),
    };
    let exists = cfg
        .auto_record_channels
        .iter()
        .any(|a| a.platform == platform && a.channel_id == id);
    if exists {
        cfg.auto_record_channels
            .retain(|a| !(a.platform == platform && a.channel_id == id));
    } else {
        cfg.auto_record_channels.push(strivo_core::config::AutoRecordEntry {
            platform,
            channel_id: id.clone(),
            channel_name: id,
            format: None,
        });
    }
    if let Err(e) = cfg.save(None) {
        return Html(format!("<span class=err>{e}</span>")).into_response();
    }
    drop(state);
    Html("ok").into_response()
}

async fn remove(
    State(state): State<AppState>,
    Path((platform, id)): Path<(String, String)>,
) -> Response {
    let mut cfg = match strivo_core::config::AppConfig::load(None) {
        Ok(c) => c,
        Err(e) => return Html(format!("<ul id=channels-list><li class=err>{e}</li></ul>")).into_response(),
    };
    cfg.auto_record_channels
        .retain(|a| !(a.platform == platform && a.channel_id == id));
    if let Err(e) = cfg.save(None) {
        return Html(format!("<ul id=channels-list><li class=err>{e}</li></ul>")).into_response();
    }
    list_partial(&state).await
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels", get(page).post(add))
        .route(
            "/channels/{platform}/{id}/auto-record",
            post(toggle_auto),
        )
        .route("/channels/{platform}/{id}", delete(remove))
}
