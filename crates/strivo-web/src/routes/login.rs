//! `POST /api/v1/auth/login` + `POST /api/v1/auth/logout` — cookie
//! session auth (W3).
//!
//! The cookie is a HMAC-SHA-256 signed token over an expiry timestamp,
//! keyed by `WebConfig.session_secret` (generated + persisted on first
//! login). Lifetime: 7 days rolling.
//!
//! API endpoints accept *either* the cookie OR the historical
//! `X-Api-Key` header (`auth::check_dual` in this module). Programmatic
//! consumers (scripts, automations) continue to use the header; the
//! browser uses the cookie after one login.

use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{ApiKey, SessionToken};
use crate::server::AppState;

pub const SESSION_COOKIE: &str = "strivo_session";

/// 7 days in seconds — rolling expiry on every login.
const SESSION_TTL_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Deserialize)]
struct LoginPayload {
    api_key: String,
}

/// `POST /api/v1/auth/login` — body `{"api_key": "<key>"}`. On success
/// sets the `strivo_session` cookie + returns the expiry timestamp.
async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginPayload>,
) -> impl IntoResponse {
    if !state.api_key.matches(&body.api_key) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid api_key"})),
        )
            .into_response();
    }

    // Lazily persist a session secret on first login. Re-uses the
    // existing config-save path so the secret survives restarts.
    let secret = match state.session_secret.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            let s = crate::auth::generate_session_secret();
            let mut cfg = match strivo_core::config::AppConfig::load(None) {
                Ok(c) => c,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": format!("config load: {e}")})),
                    )
                        .into_response();
                }
            };
            cfg.web.session_secret = Some(s.clone());
            if let Err(e) = cfg.save(None) {
                tracing::warn!("could not persist [web].session_secret: {e}");
            }
            s
        }
    };

    let token = SessionToken::new(SESSION_TTL_SECS);
    let cookie_value = token.encode(&secret);
    let cookie = format!(
        "{SESSION_COOKIE}={cookie_value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_TTL_SECS}"
    );

    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, cookie.parse().unwrap());
    (
        StatusCode::OK,
        headers,
        Json(json!({"status": "ok", "expires_at": token.expires_at})),
    )
        .into_response()
}

/// `POST /api/v1/auth/logout` — clears the cookie.
async fn logout() -> impl IntoResponse {
    let cookie = format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0");
    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, cookie.parse().unwrap());
    (StatusCode::OK, headers, Json(json!({"status": "logged out"}))).into_response()
}

/// Extract a valid session token from the request's Cookie header, if
/// any. Returns `None` for missing / malformed / expired / bad-HMAC
/// cookies — caller treats every failure as 401 if it also fails the
/// X-Api-Key check.
pub fn session_from_headers(
    headers: &HeaderMap,
    session_secret: Option<&str>,
) -> Option<SessionToken> {
    let secret = session_secret?;
    let cookie_header = headers.get("cookie").and_then(|v| v.to_str().ok())?;
    let session_pair = cookie_header
        .split(';')
        .map(|s| s.trim())
        .find(|s| s.starts_with(&format!("{SESSION_COOKIE}=")))?;
    let (_, value) = session_pair.split_once('=')?;
    SessionToken::decode_verify(value, secret)
}

/// Dual-auth check — accepts either a valid cookie session OR the
/// `X-Api-Key` header. Returns `Ok(())` on either path. The W1 routes
/// continue to call `check_key()` for the moment; new routes call
/// this instead.
pub fn check_dual(
    headers: &HeaderMap,
    api_key: &ApiKey,
    session_secret: Option<&str>,
) -> Result<(), StatusCode> {
    // Cookie path first — browser users hit this every request.
    if session_from_headers(headers, session_secret).is_some() {
        return Ok(());
    }
    // Header path — programmatic consumers.
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if api_key.matches(key) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
}
