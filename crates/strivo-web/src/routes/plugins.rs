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

/// `POST /api/v1/plugins/chapters/<recording_id>` — generate (or
/// re-generate) chapter markers for the given recording. Reads the
/// Crunchr SQLite, runs the heuristic chapter builder, caches result
/// in chapters.db, returns the chapter set inline.
async fn chapters_generate(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("chapters") { return r; }
    let crunchr_path = crunchr_db();
    if !crunchr_path.exists() {
        return Problem::not_found("crunchr DB not initialised").into_response();
    }
    let req = strivo_chapters::ChapterRequest {
        recording_id: recording_id.clone(),
        min_seconds: None,
        cos_threshold: None,
    };
    let chapters = match strivo_chapters::generate_chapters(
        &crunchr_path,
        &req,
        &strivo_chapters::KeywordTitler,
    ) {
        Ok(c) => c,
        Err(e) => return Problem::internal(format!("chapters: {e}")).into_response(),
    };
    let description = strivo_chapters::format_for_description(&chapters);
    Json(json!({
        "recording_id": recording_id,
        "chapters": chapters,
        "description": description,
    })).into_response()
}

/// `POST /api/v1/plugins/cuepoints/<recording_id>` — extract (or
/// re-extract) scene-change cuepoints for a recording. ffmpeg full
/// pass; cached in cuepoints.db keyed on (recording_id, threshold).
async fn cuepoints_generate(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("cuepoints") { return r; }
    let id = match uuid::Uuid::parse_str(&recording_id) {
        Ok(u) => u,
        Err(_) => return Problem::bad_request("recording id must be a uuid").into_response(),
    };
    // Resolve the file path via persist DB — same shape as the remux
    // endpoint. No daemon round-trip needed; we only read the row.
    let jobs_db = strivo_core::config::AppConfig::data_dir().join("jobs.db");
    let input_path = match strivo_core::recording::persist::PersistDb::open(&jobs_db) {
        Ok(db) => match db.load_recording_jobs().await {
            Ok(rows) => rows.into_iter().find(|j| j.id == id).map(|j| j.output_path),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let Some(input) = input_path else {
        return Problem::not_found("recording not found").into_response();
    };
    if !input.exists() {
        return Problem::not_found("recording file missing").into_response();
    }
    let threshold = strivo_cuepoints::DEFAULT_THRESHOLD;
    // Check the cache first.
    let store_path = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("cuepoints")
        .join("cuepoints.db");
    let store = match strivo_cuepoints::store::CuepointsStore::open(&store_path) {
        Ok(s) => s,
        Err(e) => return Problem::internal(format!("open cache: {e}")).into_response(),
    };
    if let Ok(Some(points)) = store.load(&recording_id, threshold) {
        return Json(json!({
            "recording_id": recording_id,
            "threshold": threshold,
            "points": points,
            "cached": true,
        }))
        .into_response();
    }
    let points = match strivo_cuepoints::extract_cuepoints(&input, threshold) {
        Ok(p) => p,
        Err(e) => return Problem::internal(format!("ffmpeg: {e}")).into_response(),
    };
    let set = strivo_cuepoints::CuepointSet {
        recording_id: recording_id.clone(),
        threshold,
        points: points.clone(),
    };
    let _ = store.save(&set);
    Json(json!({
        "recording_id": recording_id,
        "threshold": threshold,
        "points": points,
        "cached": false,
    }))
    .into_response()
}

/// Resolve the on-disk output path for a recording job from persist.
/// Shared by the cuepoints/clipper handlers — both need it.
async fn resolve_recording_path(recording_id: &str) -> Result<std::path::PathBuf, String> {
    let id = uuid::Uuid::parse_str(recording_id).map_err(|_| "id must be uuid".to_string())?;
    let jobs_db = strivo_core::config::AppConfig::data_dir().join("jobs.db");
    let path = match strivo_core::recording::persist::PersistDb::open(&jobs_db) {
        Ok(db) => match db.load_recording_jobs().await {
            Ok(rows) => rows.into_iter().find(|j| j.id == id).map(|j| j.output_path),
            Err(_) => None,
        },
        Err(_) => None,
    };
    path.ok_or_else(|| "recording not found".to_string())
}

#[derive(Debug, Deserialize, Default)]
struct ClipExtractPayload {
    start_sec: f32,
    duration_sec: Option<f32>,
    /// Filename stem the user wants; sanitised server-side.
    #[serde(default)]
    stem: String,
}

/// `POST /api/v1/plugins/clipper/<recording_id>/analyze` — score
/// highlight candidates for the recording. Reuses the cuepoint cache
/// (iter 4) so the analyzer is cheap on a previously-analysed VOD.
async fn clipper_analyze(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("clipper") { return r; }
    // We need cuepoints to score. If they're not cached we extract
    // them now — same path the standalone Cuepoints button uses.
    let input = match resolve_recording_path(&recording_id).await {
        Ok(p) => p,
        Err(e) => return Problem::not_found(e).into_response(),
    };
    if !input.exists() {
        return Problem::not_found("recording file missing").into_response();
    }
    let threshold = strivo_cuepoints::DEFAULT_THRESHOLD;
    let cp_store_path = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("cuepoints")
        .join("cuepoints.db");
    let cp_store = match strivo_cuepoints::store::CuepointsStore::open(&cp_store_path) {
        Ok(s) => s,
        Err(e) => return Problem::internal(format!("open cuepoints cache: {e}")).into_response(),
    };
    let cuepoints = match cp_store.load(&recording_id, threshold) {
        Ok(Some(c)) => c,
        _ => match strivo_cuepoints::extract_cuepoints(&input, threshold) {
            Ok(c) => {
                let set = strivo_cuepoints::CuepointSet {
                    recording_id: recording_id.clone(),
                    threshold,
                    points: c.clone(),
                };
                let _ = cp_store.save(&set);
                c
            }
            Err(e) => return Problem::internal(format!("cuepoints: {e}")).into_response(),
        },
    };
    let window = strivo_clipper::DEFAULT_WINDOW_SECS;
    let highlights = strivo_clipper::score_highlights(&cuepoints, window, strivo_clipper::DEFAULT_TOP_K);
    let store_path = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("clipper")
        .join("clipper.db");
    if let Ok(store) = strivo_clipper::store::ClipperStore::open(&store_path) {
        let _ = store.save_highlights(&recording_id, window, &highlights);
    }
    Json(json!({
        "recording_id": recording_id,
        "window_secs": window,
        "highlights": highlights,
    }))
    .into_response()
}

/// `POST /api/v1/plugins/clipper/<recording_id>/extract` — cut the
/// requested clip (lossless ffmpeg `-c copy`) and return the result.
async fn clipper_extract(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
    Json(body): Json<ClipExtractPayload>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("clipper") { return r; }
    let input = match resolve_recording_path(&recording_id).await {
        Ok(p) => p,
        Err(e) => return Problem::not_found(e).into_response(),
    };
    if !input.exists() {
        return Problem::not_found("recording file missing").into_response();
    }
    let dur = body.duration_sec.unwrap_or(strivo_clipper::DEFAULT_CLIP_DURATION_SECS);
    let (start, duration) = strivo_clipper::clamp_request(body.start_sec, dur, None);
    let extension = input.extension().and_then(|e| e.to_str()).unwrap_or("mkv");
    let fallback_stem = format!("clip_{:.0}", start);
    let safe_stem = sanitize_stem(if body.stem.is_empty() {
        &fallback_stem
    } else {
        &body.stem
    });
    // Land under <recording_parent>/clips/<stem>.<ext>.
    let clip_dir = input
        .parent()
        .map(|p| p.join("clips"))
        .unwrap_or_else(|| std::path::PathBuf::from("./clips"));
    let output = clip_dir.join(format!("{safe_stem}.{extension}"));
    let bytes = match strivo_clipper::extract_clip(&input, &output, start, duration) {
        Ok(n) => n,
        Err(e) => return Problem::internal(format!("ffmpeg: {e}")).into_response(),
    };
    let result = strivo_clipper::ClipResult {
        recording_id: recording_id.clone(),
        clip_path: output.to_string_lossy().to_string(),
        start_sec: start,
        duration_sec: duration,
        bytes,
    };
    let store_path = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("clipper")
        .join("clipper.db");
    if let Ok(store) = strivo_clipper::store::ClipperStore::open(&store_path) {
        let _ = store.save_clip(&result);
    }
    Json(result).into_response()
}

/// `GET /api/v1/plugins/clipper/<recording_id>/clips` — list previously
/// cut clips for a recording (powers the SPA "Cut clips" list).
async fn clipper_list_clips(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("clipper") { return r; }
    let store_path = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("clipper")
        .join("clipper.db");
    let store = match strivo_clipper::store::ClipperStore::open(&store_path) {
        Ok(s) => s,
        Err(_) => return Json(json!({ "clips": [] })).into_response(),
    };
    let clips = store.list_clips(&recording_id).unwrap_or_default();
    Json(json!({ "clips": clips })).into_response()
}

fn sanitize_stem(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "clip".to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Deserialize, Default)]
struct ThumbsPayload {
    /// Source of timestamps to sample at. "cuepoints" reuses the
    /// existing cuepoint set; "even" walks the recording at
    /// `interval_secs` boundaries; "list" takes an explicit list.
    #[serde(default = "default_thumb_source")]
    source: String,
    #[serde(default)]
    times: Vec<f32>,
    #[serde(default)]
    interval_secs: Option<f32>,
    #[serde(default)]
    facecam: Option<strivo_thumbnails::FacecamCorner>,
}

fn default_thumb_source() -> String {
    "cuepoints".to_string()
}

/// `POST /api/v1/plugins/thumbnails/<recording_id>` — generate thumbnail
/// candidates. Source = cuepoints / even / explicit-list; optional
/// facecam corner emits a 9:16 vertical crop per pick.
async fn thumbnails_generate(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
    Json(body): Json<ThumbsPayload>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("thumbnails") { return r; }
    let input = match resolve_recording_path(&recording_id).await {
        Ok(p) => p,
        Err(e) => return Problem::not_found(e).into_response(),
    };
    if !input.exists() {
        return Problem::not_found("recording file missing").into_response();
    }
    // Build the timestamp list.
    let timestamps: Vec<f32> = match body.source.as_str() {
        "list" => body.times.clone(),
        "even" => {
            let interval = body.interval_secs.unwrap_or(600.0).max(15.0);
            // ffprobe duration to know how far to walk.
            let duration = probe_duration(&input).unwrap_or(3600.0);
            let mut out = Vec::new();
            let mut t = 0.0_f32;
            while t < duration {
                out.push(t);
                t += interval;
            }
            out
        }
        _ /* "cuepoints" */ => {
            let cp_path = strivo_core::config::AppConfig::data_dir()
                .join("plugins")
                .join("cuepoints")
                .join("cuepoints.db");
            let store = strivo_cuepoints::store::CuepointsStore::open(&cp_path).ok();
            let cps = store
                .and_then(|s| s.load(&recording_id, strivo_cuepoints::DEFAULT_THRESHOLD).ok().flatten())
                .unwrap_or_default();
            if cps.is_empty() {
                // Fall back to a handful of even samples so the user gets *something*.
                let duration = probe_duration(&input).unwrap_or(3600.0);
                let n = 8;
                (0..n).map(|i| duration * (i as f32 + 0.5) / n as f32).collect()
            } else {
                cps.iter().map(|c| c.time_sec).collect()
            }
        }
    };
    // Cap to a sensible upper bound — running ffmpeg 200 times stalls
    // the UI thread. SPA can request a smaller batch if it wants more.
    let timestamps: Vec<f32> = timestamps.into_iter().take(24).collect();
    if timestamps.is_empty() {
        return Problem::bad_request("no timestamps to sample").into_response();
    }
    // ffprobe resolution for cropping.
    let (w, h) = probe_resolution(&input).unwrap_or((1920, 1080));

    let out_dir = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("thumbnails")
        .join(&recording_id);
    let stem = "candidate";
    let opts = strivo_thumbnails::GenerateOptions {
        timestamps,
        out_dir,
        stem: stem.to_string(),
        facecam: body.facecam,
    };
    let result = match strivo_thumbnails::generate_candidates(&input, (w, h), &opts, &recording_id) {
        Ok(r) => r,
        Err(e) => return Problem::internal(format!("thumbnails: {e}")).into_response(),
    };

    let store_path = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("thumbnails")
        .join("thumbnails.db");
    if let Ok(store) = strivo_thumbnails::store::ThumbnailsStore::open(&store_path) {
        let _ = store.save(&recording_id, stem, &result.candidates);
    }
    Json(json!({
        "recording_id": recording_id,
        "candidates": result.candidates,
    }))
    .into_response()
}

/// Shell out to ffprobe for the duration in seconds.
fn probe_duration(input: &std::path::Path) -> Option<f32> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input)
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    s.trim().parse().ok()
}

/// Shell out to ffprobe for the video resolution.
fn probe_resolution(input: &std::path::Path) -> Option<(u32, u32)> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height",
            "-of", "csv=p=0",
        ])
        .arg(input)
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let parts: Vec<&str> = s.trim().split(',').collect();
    if parts.len() == 2 {
        Some((parts[0].parse().ok()?, parts[1].parse().ok()?))
    } else {
        None
    }
}

/// `GET /api/v1/plugins/thumbnails/<recording_id>/<stem>` — list the
/// cached candidate set so a page reload doesn't lose state.
async fn thumbnails_list(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path((recording_id, stem)): Path<(String, String)>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("thumbnails") { return r; }
    let store_path = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("thumbnails")
        .join("thumbnails.db");
    let store = match strivo_thumbnails::store::ThumbnailsStore::open(&store_path) {
        Ok(s) => s,
        Err(_) => return Json(json!({ "candidates": [] })).into_response(),
    };
    let candidates = store.load(&recording_id, &stem).ok().flatten().unwrap_or_default();
    Json(json!({ "candidates": candidates })).into_response()
}

/// `GET /api/v1/plugins/thumbnails/file?p=<absolute_path>` — serve a
/// generated thumbnail file. We refuse anything outside the
/// thumbnails data dir so the route can't be used as a generic
/// file-read.
async fn thumbnails_serve_file(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(q): Query<ThumbFilePayload>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("thumbnails") { return r; }
    let root = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("thumbnails");
    let path = std::path::PathBuf::from(&q.p);
    // Canonicalise both so symlinks can't escape.
    let canon_root = match root.canonicalize() {
        Ok(p) => p,
        Err(_) => return Problem::internal("thumb root").into_response(),
    };
    let canon_path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return Problem::not_found("thumb").into_response(),
    };
    if !canon_path.starts_with(&canon_root) {
        return Problem::bad_request("path outside thumbnails dir").into_response();
    }
    let body = match std::fs::read(&canon_path) {
        Ok(b) => b,
        Err(_) => return Problem::not_found("thumb").into_response(),
    };
    let mime = if canon_path.extension().and_then(|e| e.to_str()) == Some("jpg") {
        "image/jpeg"
    } else {
        "application/octet-stream"
    };
    ([(axum::http::header::CONTENT_TYPE, mime)], body).into_response()
}

#[derive(Debug, Deserialize)]
struct ThumbFilePayload {
    p: String,
}

#[derive(Debug, Deserialize)]
struct InsightsCompareQuery {
    /// Comma-separated list of recording UUIDs (exactly two; first two
    /// taken if more are supplied).
    recs: String,
    #[serde(default = "default_compare_limit")]
    limit: u32,
    #[serde(default)]
    include_stopwords: bool,
}

fn default_compare_limit() -> u32 {
    50
}

/// `GET /api/v1/plugins/insights/compare?recs=A,B` — pull each
/// recording's top-N word list from Crunchr and run the comparator.
/// Returns the shared / only_a / only_b sets + Jaccard score.
async fn insights_compare(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(q): Query<InsightsCompareQuery>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("insights") { return r; }
    let ids: Vec<String> = q
        .recs
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if ids.len() < 2 {
        return Problem::bad_request("recs= must list at least two recording ids").into_response();
    }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Problem::not_found("crunchr has no data yet").into_response();
    };
    let limit = q.limit.clamp(10, 500) as usize;
    let fetch = |rid: &str| -> Vec<strivo_insights_compare::WordCount> {
        strivo_plugins::insights::frequency::top_words_for_recording(
            &conn,
            rid,
            limit,
            q.include_stopwords,
        )
        .ok()
        .unwrap_or_default()
        .into_iter()
        .map(|w| strivo_insights_compare::WordCount {
            word: w.word,
            count: w.count as u64,
        })
        .collect()
    };
    let a = fetch(&ids[0]);
    let b = fetch(&ids[1]);
    let cmp = strivo_insights_compare::compare_words(&a, &b);
    Json(json!({
        "recording_a": ids[0],
        "recording_b": ids[1],
        "limit": limit,
        "comparison": cmp,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct RetentionQuery {
    #[serde(default = "default_bucket_secs")]
    bucket_secs: f32,
}

fn default_bucket_secs() -> f32 {
    30.0
}

/// `GET /api/v1/plugins/insights/retention/<recording_id>` — bucket
/// transcript activity + cuepoint density into a retention curve.
async fn insights_retention(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
    Query(q): Query<RetentionQuery>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("insights") { return r; }
    // Pull Crunchr segments.
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Problem::not_found("crunchr has no data yet").into_response();
    };
    let detail = match strivo_plugins::crunchr::db::recording_detail(&conn, &recording_id) {
        Ok(Some(d)) => d,
        Ok(None) => return Problem::not_found("recording not transcribed").into_response(),
        Err(e) => return Problem::internal(e.to_string()).into_response(),
    };
    let segments: Vec<strivo_insights_compare::Segment> = detail
        .segments
        .iter()
        .map(|s| {
            // Word count proxy: split on whitespace. Cheap, deterministic.
            let words = s.text.split_whitespace().count() as u32;
            strivo_insights_compare::Segment {
                start_sec: s.start_sec as f32,
                end_sec: s.end_sec as f32,
                word_count: words,
            }
        })
        .collect();
    // Pull cuepoints for the same recording (best-effort).
    let cp_path = strivo_core::config::AppConfig::data_dir()
        .join("plugins")
        .join("cuepoints")
        .join("cuepoints.db");
    let cuepoint_times: Vec<f32> = strivo_cuepoints::store::CuepointsStore::open(&cp_path)
        .ok()
        .and_then(|store| {
            store
                .load(&recording_id, strivo_cuepoints::DEFAULT_THRESHOLD)
                .ok()
                .flatten()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.time_sec)
        .collect();
    // Duration: max segment end_sec; clamp to a sane floor so even
    // a 3-second test recording yields a curve.
    let duration = segments
        .iter()
        .map(|s| s.end_sec)
        .fold(0.0_f32, f32::max)
        .max(q.bucket_secs * 2.0);
    let bucket = q.bucket_secs.max(5.0);
    let curve = strivo_insights_compare::compute_retention(
        &segments,
        &cuepoint_times,
        duration,
        bucket,
    );
    Json(json!({
        "recording_id": recording_id,
        "duration_sec": duration,
        "bucket_secs": bucket,
        "points": curve,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct CaptionsQuery {
    /// Output format. One of `srt`, `vtt`, `txt`.
    #[serde(default = "default_captions_fmt")]
    fmt: String,
    /// Optional target language. Default `en` = identity.
    #[serde(default = "default_captions_lang")]
    lang: String,
}

fn default_captions_fmt() -> String { "srt".to_string() }
fn default_captions_lang() -> String { "en".to_string() }

/// `GET /api/v1/plugins/captions/<recording_id>?fmt=srt&lang=en` —
/// emit a caption file for the recording in the requested format. The
/// `lang` knob is currently routed through an identity translator; the
/// pluggable `Translator` trait will get a real backend in a follow-up.
async fn captions_export(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
    Query(q): Query<CaptionsQuery>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("captions") { return r; }
    let Some(conn) = open_ro(&crunchr_db()) else {
        return Problem::not_found("crunchr has no data yet").into_response();
    };
    let detail = match strivo_plugins::crunchr::db::recording_detail(&conn, &recording_id) {
        Ok(Some(d)) => d,
        Ok(None) => return Problem::not_found("recording not transcribed").into_response(),
        Err(e) => return Problem::internal(e.to_string()).into_response(),
    };
    let segments: Vec<strivo_captions::Segment> = detail
        .segments
        .iter()
        .map(|s| strivo_captions::Segment {
            start_sec: s.start_sec as f32,
            end_sec: s.end_sec as f32,
            text: s.text.clone(),
            speaker: s.speaker.clone(),
        })
        .collect();
    // Apply translation. Today only IdentityTranslator ships; a future
    // iteration registers real backends (NLLB / Argos / OpenAI).
    let translator = strivo_captions::IdentityTranslator;
    let translated = match strivo_captions::apply_translation(&segments, &translator) {
        Ok(out) => out,
        Err(e) => return Problem::internal(format!("translate: {e}")).into_response(),
    };
    let (body, mime, ext) = match q.fmt.as_str() {
        "vtt" => (strivo_captions::to_vtt(&translated), "text/vtt", "vtt"),
        "txt" => (strivo_captions::to_txt(&translated), "text/plain", "txt"),
        _ /* srt */ => (
            strivo_captions::to_srt(&translated),
            "application/x-subrip",
            "srt",
        ),
    };
    let safe = recording_id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_', "_");
    let filename = format!("{safe}.{}.{ext}", q.lang);
    (
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (
                axum::http::header::CONTENT_DISPOSITION,
                Box::leak(format!("attachment; filename=\"{filename}\"").into_boxed_str()),
            ),
        ],
        body,
    )
        .into_response()
}

/// `GET /api/v1/plugins/multitrack/<recording_id>` — list the audio
/// tracks present in the recording. Pure ffprobe call — fast.
async fn multitrack_list(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("multitrack") { return r; }
    let input = match resolve_recording_path(&recording_id).await {
        Ok(p) => p,
        Err(e) => return Problem::not_found(e).into_response(),
    };
    if !input.exists() {
        return Problem::not_found("recording file missing").into_response();
    }
    match strivo_multitrack::probe_audio_tracks(&input) {
        Ok(tracks) => Json(json!({
            "recording_id": recording_id,
            "tracks": tracks,
        }))
        .into_response(),
        Err(e) => Problem::internal(format!("ffprobe: {e}")).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct MultitrackExtractPayload {
    /// Stream index of the track to extract (matches AudioTrack.index).
    track_index: u32,
    /// Optional filename stem; defaults to "track_<idx>".
    #[serde(default)]
    stem: String,
    /// Optional output extension — overrides the source codec's
    /// natural extension when set. Caller is responsible for picking
    /// something the codec actually fits into.
    #[serde(default)]
    ext: String,
}

/// `POST /api/v1/plugins/multitrack/<recording_id>/extract` — cut a
/// single audio track to a standalone file in `<recording>/tracks/`.
async fn multitrack_extract(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
    Json(body): Json<MultitrackExtractPayload>,
) -> impl IntoResponse {
    if authed(&headers, &state).is_err() {
        return Problem::unauthorized().into_response();
    }
    if let Err(r) = gate_pro("multitrack") { return r; }
    let input = match resolve_recording_path(&recording_id).await {
        Ok(p) => p,
        Err(e) => return Problem::not_found(e).into_response(),
    };
    if !input.exists() {
        return Problem::not_found("recording file missing").into_response();
    }
    // Pick a sensible extension. Default to the source file's extension
    // since `-c copy` keeps the codec; user can override via the payload.
    let src_ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mka");
    let ext = if body.ext.is_empty() { src_ext } else { body.ext.as_str() };
    let stem = if body.stem.is_empty() {
        format!("track_{}", body.track_index)
    } else {
        body.stem.replace(
            |c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-',
            "_",
        )
    };
    let out_dir = input
        .parent()
        .map(|p| p.join("tracks"))
        .unwrap_or_else(|| std::path::PathBuf::from("./tracks"));
    let output = out_dir.join(format!("{stem}.{ext}"));
    let bytes = match strivo_multitrack::extract_track(&input, body.track_index, &output) {
        Ok(b) => b,
        Err(e) => return Problem::internal(format!("ffmpeg: {e}")).into_response(),
    };
    Json(json!({
        "recording_id": recording_id,
        "track_index": body.track_index,
        "output_path": output.to_string_lossy(),
        "bytes": bytes,
    }))
    .into_response()
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

    // Per-plugin data_dir surfaces in the response so the SPA can show
    // a "data" hint without the user having to know the layout (M6).
    let crunchr_data = crunchr_db()
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let archiver_data = archiver_db()
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let viewguard_data = viewguard_db()
        .as_deref()
        .and_then(|p| p.parent().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_default();

    let crunchr_conn = open_ro(&crunchr_db());
    let crunchr = match &crunchr_conn {
        Some(c) => json!({
            "name": "crunchr",
            "display": "Crunchr",
            "description": "AI transcription, diarization & analysis",
            "available": crunchr_ok,
            "pro": true,
            "entitled": crunchr_ok,
            "data_dir": crunchr_data,
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
            "data_dir": crunchr_data,
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
            "data_dir": archiver_data,
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
            "data_dir": viewguard_data,
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
        .route("/api/v1/plugins/chapters/{id}", axum::routing::post(chapters_generate))
        .route("/api/v1/plugins/cuepoints/{id}", axum::routing::post(cuepoints_generate))
        .route("/api/v1/plugins/clipper/{id}/analyze", axum::routing::post(clipper_analyze))
        .route("/api/v1/plugins/clipper/{id}/extract", axum::routing::post(clipper_extract))
        .route("/api/v1/plugins/clipper/{id}/clips", get(clipper_list_clips))
        .route("/api/v1/plugins/thumbnails/{id}", axum::routing::post(thumbnails_generate))
        .route("/api/v1/plugins/thumbnails/{id}/{stem}", get(thumbnails_list))
        .route("/api/v1/plugins/thumbnails/file", get(thumbnails_serve_file))
        .route("/api/v1/plugins/insights/compare", get(insights_compare))
        .route("/api/v1/plugins/insights/retention/{id}", get(insights_retention))
        .route("/api/v1/plugins/captions/{id}", get(captions_export))
        .route("/api/v1/plugins/multitrack/{id}", get(multitrack_list))
        .route("/api/v1/plugins/multitrack/{id}/extract", axum::routing::post(multitrack_extract))
}
