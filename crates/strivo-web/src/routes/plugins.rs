//! `/api/v1/plugins/*` read surface for the first-party plugins.
//!
//! The web server is a separate process from the daemon (where the plugin
//! registry actually lives), so action dispatch — "Re-transcribe", "Re-archive
//! channel" — flows over IPC via [`crate::routes::api`]'s `plugin_rpc`. But the
//! plugins also persist everything they produce to SQLite under
//! `<data_dir>/plugins/<name>/`. For *reads* we open those DBs read-only and
//! reuse the plugin crate's query functions directly, which avoids inventing a
//! query/response IPC protocol for what is fundamentally a co-located file read.
//!
//! Every endpoint is auth-gated (session cookie or `X-Api-Key`). When a
//! plugin's DB doesn't exist yet — the plugin has produced nothing — the
//! handlers return `{ "available": false, … }` with an empty payload so the
//! SPA can render a friendly empty state instead of an error.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::problem::Problem;
use crate::server::AppState;

fn authed(headers: &HeaderMap, state: &AppState) -> Result<(), StatusCode> {
    crate::routes::login::check_dual(headers, &state.api_key, &state.session_secret)
}

// ── DB path resolution ───────────────────────────────────────────────

fn plugins_root() -> PathBuf {
    strivo_core::config::AppConfig::data_dir().join("plugins")
}

fn crunchr_db() -> PathBuf {
    plugins_root().join("crunchr").join("crunchr.db")
}

fn archiver_db() -> PathBuf {
    plugins_root().join("archiver").join("archiver.db")
}

/// Viewguard's `init` joins `plugins/viewguard` onto a data_dir that already
/// ends in `plugins/viewguard`, so the live DB nests twice. Prefer that path
/// but fall back to the un-nested location so this keeps working if the plugin
/// ever corrects the join.
fn viewguard_db() -> Option<PathBuf> {
    let nested = plugins_root()
        .join("viewguard")
        .join("plugins")
        .join("viewguard")
        .join("viewguard.db");
    let flat = plugins_root().join("viewguard").join("viewguard.db");
    if nested.exists() {
        Some(nested)
    } else if flat.exists() {
        Some(flat)
    } else {
        None
    }
}

/// Open a plugin DB read-only. Returns None when the file is absent (plugin
/// idle) so callers can serve an empty payload.
fn open_ro(path: &std::path::Path) -> Option<Connection> {
    if !path.exists() {
        return None;
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
}

/// Returns Err(402) when `name` is a Pro plugin and this machine is not
/// entitled. Free plugins always Ok. The check is centralised here so
/// every data route shares the same gate without forgetting one.
fn gate_pro(name: &str) -> Result<(), axum::response::Response> {
    if strivo_core::licence::gate::is_entitled(name) {
        return Ok(());
    }
    Err(Problem::payment_required(format!(
        "{name} is a Strivo Pro plugin — activate or start a 3-day trial from the Plugins page."
    ))
    .into_response())
}

// ── Plugin index ─────────────────────────────────────────────────────

/// `GET /api/v1/plugins` — the four data-backed first-party plugins with a
/// readiness flag + rollup counts, so the hub can show "12 recordings" etc.
async fn index(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }

    // Pro-plugin entitlement decides whether each first-party plugin
    // unlocks. We still include locked Pro plugins in the response so
    // the SPA can render them with a lock badge + upgrade CTA — hiding
    // them entirely would make the upgrade card feel disconnected.
    let pro_entitled = |name: &str| strivo_core::licence::gate::is_entitled(name);
    let crunchr_ok = pro_entitled("crunchr");
    let archiver_ok = pro_entitled("archiver");
    let viewguard_ok = pro_entitled("viewguard");
    let insights_ok = pro_entitled("insights");

    let crunchr_conn = open_ro(&crunchr_db());
    let crunchr = match &crunchr_conn {
        Some(c) => json!({
            "name": "crunchr",
            "display": "Crunchr",
            "description": "AI transcription, diarization & analysis",
            "available": crunchr_ok,
            "pro": true,
            "entitled": crunchr_ok,
            "stats": {
                "recordings": count(c, "SELECT COUNT(*) FROM videos"),
                "analyzed": count(c, "SELECT COUNT(*) FROM video_analysis"),
            },
            "verbs": [
                // Verb name is the dispatch key (matches the plugin's
                // `PluginCommand::item("Re-transcribe", …)`); label is the
                // user-facing string the SPA renders.
                { "verb": "Re-transcribe", "scope": "recording", "label": "Generate subtitles" },
            ],
        }),
        None => json!({
            "name": "crunchr", "display": "Crunchr",
            "description": "AI transcription, diarization & analysis",
            "available": false, "pro": true, "entitled": crunchr_ok, "stats": {}, "verbs": []
        }),
    };

    let insights = match &crunchr_conn {
        Some(c) => json!({
            "name": "insights",
            "display": "Insights",
            "description": "Word frequency, speaker airtime, topics & sentiment",
            "available": insights_ok,
            "pro": true,
            "entitled": insights_ok,
            "stats": {
                "words": count(c, "SELECT COUNT(DISTINCT word) FROM word_frequency"),
                "topics_videos": count(c, "SELECT COUNT(*) FROM video_analysis WHERE topics IS NOT NULL AND topics != ''"),
            },
            "verbs": [],
        }),
        None => json!({
            "name": "insights", "display": "Insights",
            "description": "Word frequency, speaker airtime, topics & sentiment",
            "available": false, "pro": true, "entitled": insights_ok, "stats": {}, "verbs": []
        }),
    };

    let archiver = match open_ro(&archiver_db()) {
        Some(c) => json!({
            "name": "archiver",
            "display": "Archiver",
            "description": "Back-catalog VOD pulls & download tracking",
            "available": archiver_ok,
            "pro": true,
            "entitled": archiver_ok,
            "stats": {
                "channels": count(&c, "SELECT COUNT(*) FROM channels"),
                "videos": count(&c, "SELECT COUNT(*) FROM videos"),
                "downloaded": count(&c, "SELECT COUNT(*) FROM videos WHERE downloaded"),
            },
            "verbs": [
                { "verb": "Re-archive channel", "scope": "recording", "label": "Re-archive channel" },
            ],
        }),
        None => json!({
            "name": "archiver", "display": "Archiver",
            "description": "Back-catalog VOD pulls & download tracking",
            "available": false, "pro": true, "entitled": archiver_ok, "stats": {}, "verbs": []
        }),
    };

    let viewguard = match viewguard_db().as_deref().and_then(open_ro) {
        Some(c) => json!({
            "name": "viewguard",
            "display": "Viewguard",
            "description": "Viewbot fraud detection — verdicts & viewer signals",
            "available": viewguard_ok,
            "pro": true,
            "entitled": viewguard_ok,
            "stats": {
                "verdicts": count(&c, "SELECT COUNT(DISTINCT channel_id) FROM verdicts"),
                "samples": count(&c, "SELECT COUNT(*) FROM samples"),
            },
            "verbs": [],
        }),
        None => json!({
            "name": "viewguard", "display": "Viewguard",
            "description": "Viewbot fraud detection — verdicts & viewer signals",
            "available": false, "pro": true, "entitled": viewguard_ok, "stats": {}, "verbs": []
        }),
    };

    // Hide locked Pro plugins entirely — the upgrade card on the same
    // page carries the unlock story, so surfacing a dimmed row would
    // just be noise. Free plugins (none today) would still appear.
    let plugins: Vec<Value> = [
        (crunchr_ok, crunchr),
        (insights_ok, insights),
        (archiver_ok, archiver),
        (viewguard_ok, viewguard),
    ]
    .into_iter()
    .filter_map(|(ok, v)| if ok { Some(v) } else { None })
    .collect();
    Json(json!({ "plugins": plugins })).into_response()
}

// ── Crunchr ──────────────────────────────────────────────────────────

async fn crunchr_recordings(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("crunchr") { return r; }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Json(json!({ "available": false, "recordings": [] })).into_response();
    };
    match strivo_plugins::crunchr::db::list_videos(&conn) {
        Ok(vids) => {
            let items: Vec<Value> = vids
                .into_iter()
                .map(|v| {
                    json!({
                        "recording_id": v.recording_id,
                        "channel_name": v.channel_name,
                        "title": v.title,
                        "status": v.status,
                        "segment_count": v.segment_count,
                        "has_analysis": v.has_analysis,
                        "created_at": v.created_at,
                    })
                })
                .collect();
            Json(json!({ "available": true, "recordings": items })).into_response()
        }
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

async fn crunchr_recording(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("crunchr") { return r; }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Problem::not_found("crunchr has no data yet").into_response();
    };
    match strivo_plugins::crunchr::db::recording_detail(&conn, &id) {
        Ok(Some(d)) => {
            let segments: Vec<Value> = d
                .segments
                .into_iter()
                .map(|s| {
                    json!({
                        "index": s.index,
                        "start_sec": s.start_sec,
                        "end_sec": s.end_sec,
                        "text": s.text,
                        "speaker": s.speaker,
                        "confidence": s.confidence,
                    })
                })
                .collect();
            Json(json!({
                "recording_id": d.recording_id,
                "channel_name": d.channel_name,
                "title": d.title,
                "status": d.status,
                "summary": d.summary,
                "topics": d.topics,
                "sentiment": d.sentiment,
                "segments": segments,
            }))
            .into_response()
        }
        Ok(None) => Problem::not_found("recording not transcribed").into_response(),
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
}

async fn crunchr_search(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("crunchr") { return r; }
    let query = q.q.trim();
    if query.is_empty() {
        return Json(json!({ "results": [] })).into_response();
    }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Json(json!({ "available": false, "results": [] })).into_response();
    };
    match strivo_plugins::crunchr::db::fts_search(&conn, query, 50) {
        Ok(results) => {
            let items: Vec<Value> = results
                .into_iter()
                .map(|r| {
                    json!({
                        "chunk_id": r.chunk_id,
                        "video_title": r.video_title,
                        "channel_name": r.channel_name,
                        "snippet": r.snippet,
                        "start_sec": r.start_sec,
                        "end_sec": r.end_sec,
                        "score": r.score,
                    })
                })
                .collect();
            Json(json!({ "available": true, "results": items })).into_response()
        }
        // FTS rejects some punctuation as a malformed MATCH expression; treat
        // that as "no results" rather than a 500 so typing mid-query is calm.
        Err(_) => Json(json!({ "available": true, "results": [] })).into_response(),
    }
}

// ── Archiver ─────────────────────────────────────────────────────────

async fn archiver_channels(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("archiver") { return r; }
    let Some(conn) = open_ro(&archiver_db()) else {
        return Json(json!({ "available": false, "channels": [] })).into_response();
    };
    match strivo_plugins::archiver::db::list_channels(&conn) {
        Ok(chans) => {
            let items: Vec<Value> = chans
                .into_iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "name": c.name,
                        "url": c.url,
                        "platform": c.platform,
                        "archive_dir": c.archive_dir,
                        "last_scan": c.last_scan,
                        "video_count": c.video_count,
                        "downloaded_count": c.downloaded_count,
                    })
                })
                .collect();
            Json(json!({ "available": true, "channels": items })).into_response()
        }
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

async fn archiver_videos(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("archiver") { return r; }
    let Some(conn) = open_ro(&archiver_db()) else {
        return Json(json!({ "available": false, "videos": [] })).into_response();
    };
    match strivo_plugins::archiver::db::list_videos(&conn, channel_id) {
        Ok(vids) => {
            let items: Vec<Value> = vids
                .into_iter()
                .map(|v| {
                    json!({
                        "video_id": v.video_id,
                        "title": v.title,
                        "upload_date": v.upload_date,
                        "duration": v.duration,
                        "playlist": v.playlist,
                        "downloaded": v.downloaded,
                    })
                })
                .collect();
            Json(json!({ "available": true, "videos": items })).into_response()
        }
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

// ── Viewguard ────────────────────────────────────────────────────────

async fn viewguard_verdicts(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("viewguard") { return r; }
    let Some(conn) = viewguard_db().as_deref().and_then(open_ro) else {
        return Json(json!({ "available": false, "verdicts": [] })).into_response();
    };
    match strivo_plugins::viewguard::store::all_verdicts(&conn) {
        Ok(verdicts) => {
            let items: Vec<Value> = verdicts
                .into_iter()
                .map(|v| {
                    let contributors: Value = serde_json::from_str(&v.contributors_json)
                        .unwrap_or(Value::Null);
                    json!({
                        "channel_id": v.channel_id,
                        "stream_started_at": v.stream_started_at.to_rfc3339(),
                        "stream_ended_at": v.stream_ended_at.map(|t| t.to_rfc3339()),
                        "final_score": v.final_score,
                        "band": v.band,
                        "contributors": contributors,
                    })
                })
                .collect();
            Json(json!({ "available": true, "verdicts": items })).into_response()
        }
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

async fn viewguard_samples(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(channel_id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("viewguard") { return r; }
    let Some(conn) = viewguard_db().as_deref().and_then(open_ro) else {
        return Json(json!({ "available": false, "samples": [] })).into_response();
    };
    match strivo_plugins::viewguard::store::samples_for(&conn, &channel_id, 240) {
        Ok(samples) => {
            let items: Vec<Value> = samples
                .into_iter()
                .map(|s| json!({ "ts": s.ts, "viewers": s.viewers }))
                .collect();
            Json(json!({ "available": true, "samples": items })).into_response()
        }
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

// ── Insights (read crunchr.db) ───────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WordsQuery {
    /// "global" (default) or "recording".
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    recording: Option<String>,
    #[serde(default)]
    stopwords: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn insights_words(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(q): Query<WordsQuery>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("insights") { return r; }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Json(json!({ "available": false, "words": [] })).into_response();
    };
    let include_stopwords = q.stopwords.unwrap_or(false);
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let result = if q.scope.as_deref() == Some("recording") {
        match q.recording.as_deref() {
            Some(rec) => {
                strivo_plugins::insights::frequency::top_words_for_recording(
                    &conn,
                    rec,
                    limit,
                    include_stopwords,
                )
            }
            None => return Problem::bad_request("recording scope needs ?recording=<id>").into_response(),
        }
    } else {
        strivo_plugins::insights::frequency::top_words_global(&conn, limit, include_stopwords)
    };
    match result {
        Ok(rows) => {
            let items: Vec<Value> = rows
                .into_iter()
                .map(|r| json!({ "word": r.word, "count": r.count }))
                .collect();
            Json(json!({ "available": true, "words": items })).into_response()
        }
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

async fn insights_topics(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("insights") { return r; }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Json(json!({ "available": false, "topics": [] })).into_response();
    };
    match strivo_plugins::insights::topics::cross_recording_topics(&conn) {
        Ok(mut rows) => {
            rows.sort_by(|a, b| b.count.cmp(&a.count));
            let items: Vec<Value> = rows
                .into_iter()
                .map(|t| {
                    json!({
                        "topic": t.topic,
                        "count": t.count,
                        "first_seen": t.first_seen,
                        "last_seen": t.last_seen,
                    })
                })
                .collect();
            Json(json!({ "available": true, "topics": items })).into_response()
        }
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

async fn insights_speakers(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("insights") { return r; }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Json(json!({ "available": false, "speakers": [], "sentiment": null })).into_response();
    };
    let airtime = strivo_plugins::insights::speakers::airtime_for_recording(&conn, &id);
    let sentiment = strivo_plugins::insights::speakers::sentiment_for_recording(&conn, &id);
    match airtime {
        Ok(rows) => {
            let speakers: Vec<Value> = rows
                .into_iter()
                .map(|s| {
                    json!({ "speaker": s.speaker, "seconds": s.seconds, "segments": s.segments })
                })
                .collect();
            let sentiment_label = sentiment
                .ok()
                .flatten()
                .map(|p| p.label.label().to_string());
            Json(json!({
                "available": true,
                "speakers": speakers,
                "sentiment": sentiment_label,
            }))
            .into_response()
        }
        Err(e) => Problem::internal(e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct ExportQuery {
    /// "csv" (default) or "json".
    #[serde(default)]
    fmt: Option<String>,
    #[serde(default)]
    stopwords: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /api/v1/plugins/insights/export` — global word frequencies as a
/// downloadable CSV or JSON. Built inline (not via the plugin's disk export)
/// so the browser gets a direct attachment.
async fn insights_export(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("insights") { return r; }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Problem::not_found("crunchr has no data yet").into_response();
    };
    let include_stopwords = q.stopwords.unwrap_or(false);
    let limit = q.limit.unwrap_or(200).clamp(1, 5000);
    let rows = match strivo_plugins::insights::frequency::top_words_global(
        &conn,
        limit,
        include_stopwords,
    ) {
        Ok(r) => r,
        Err(e) => return Problem::internal(e.to_string()).into_response(),
    };

    let as_json = q.fmt.as_deref() == Some("json");
    if as_json {
        let body: Vec<Value> = rows
            .into_iter()
            .map(|r| json!({ "word": r.word, "count": r.count }))
            .collect();
        (
            [
                (header::CONTENT_TYPE, "application/json"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"word-frequency.json\"",
                ),
            ],
            serde_json::to_string_pretty(&body).unwrap_or_default(),
        )
            .into_response()
    } else {
        let mut csv = String::from("word,count\n");
        for r in rows {
            // Quote the word and escape embedded quotes — words are user
            // speech, so commas/quotes are possible.
            let w = r.word.replace('"', "\"\"");
            csv.push_str(&format!("\"{}\",{}\n", w, r.count));
        }
        (
            [
                (header::CONTENT_TYPE, "text/csv"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"word-frequency.csv\"",
                ),
            ],
            csv,
        )
            .into_response()
    }
}

// ── Captions sidecar (Crunchr → WebVTT) ──────────────────────────────

fn fmt_vtt_time(sec: f64) -> String {
    let sec = sec.max(0.0);
    let total_ms = (sec * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let s = (total_ms / 1000) % 60;
    let m = (total_ms / 60_000) % 60;
    let h = total_ms / 3_600_000;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Escape `<` / `>` / `&` for embedding in a VTT cue body. `<v Name>` is a
/// real VTT voice tag and must NOT be escaped — we emit that ourselves.
fn vtt_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `GET /api/v1/recordings/{id}/captions.vtt` — Crunchr's transcript segments
/// rendered as WebVTT so the in-app player's `<track>` picks them up. Returns
/// 404 when Crunchr hasn't transcribed the recording yet (so the player's
/// HEAD probe correctly hides the CC button).
async fn recording_captions(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Problem::not_found("crunchr has no data yet").into_response();
    };
    let detail = match strivo_plugins::crunchr::db::recording_detail(&conn, &id) {
        Ok(Some(d)) => d,
        Ok(None) => return Problem::not_found("recording not transcribed").into_response(),
        Err(e) => return Problem::internal(e.to_string()).into_response(),
    };
    if detail.segments.is_empty() {
        return Problem::not_found("no segments").into_response();
    }
    let mut body = String::from("WEBVTT\n\n");
    for seg in &detail.segments {
        body.push_str(&format!(
            "{} --> {}\n",
            fmt_vtt_time(seg.start_sec),
            fmt_vtt_time(seg.end_sec)
        ));
        let text = vtt_escape(seg.text.trim());
        if let Some(spk) = seg.speaker.as_deref().filter(|s| !s.is_empty()) {
            body.push_str(&format!("<v {}>{}\n\n", vtt_escape(spk), text));
        } else {
            body.push_str(&format!("{text}\n\n"));
        }
    }
    (
        [
            (header::CONTENT_TYPE, "text/vtt; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/plugins", get(index))
        .route("/api/v1/plugins/crunchr/recordings", get(crunchr_recordings))
        .route("/api/v1/plugins/crunchr/recordings/{id}", get(crunchr_recording))
        .route("/api/v1/plugins/crunchr/search", get(crunchr_search))
        .route("/api/v1/plugins/archiver/channels", get(archiver_channels))
        .route(
            "/api/v1/plugins/archiver/channels/{channel_id}/videos",
            get(archiver_videos),
        )
        .route("/api/v1/plugins/viewguard/verdicts", get(viewguard_verdicts))
        .route(
            "/api/v1/plugins/viewguard/channels/{channel_id}/samples",
            get(viewguard_samples),
        )
        .route("/api/v1/plugins/insights/words", get(insights_words))
        .route("/api/v1/plugins/insights/topics", get(insights_topics))
        .route(
            "/api/v1/plugins/insights/recordings/{id}/speakers",
            get(insights_speakers),
        )
        .route("/api/v1/plugins/insights/export", get(insights_export))
        .route("/api/v1/recordings/{id}/captions.vtt", get(recording_captions))
}
