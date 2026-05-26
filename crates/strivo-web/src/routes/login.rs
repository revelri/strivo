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

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::header::{RETRY_AFTER, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{ApiKey, SessionToken};
use crate::server::AppState;

pub const SESSION_COOKIE: &str = "strivo_session";
/// `__Host-` prefixed name used over HTTPS (e.g. behind `tailscale serve`).
/// The prefix is browser-enforced: the cookie MUST carry `Secure` and
/// `Path=/` with no `Domain`, so it can't be set by a non-secure or
/// differently-scoped origin. Plain loopback HTTP can't use it (no
/// `Secure`), so we fall back to the unprefixed name there.
pub const SESSION_COOKIE_HOST: &str = "__Host-strivo_session";

/// 7 days in seconds — rolling expiry on every login.
const SESSION_TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// True when the request reached us over HTTPS. The strivo process always
/// terminates plain HTTP (loopback, or behind `tailscale serve` which
/// terminates TLS and forwards), so the only HTTPS signal is the proxy's
/// `X-Forwarded-Proto`. Trusting it here is safe: it can only make the
/// cookie *more* restrictive (`Secure` + `__Host-`); a spoofed `https`
/// over real HTTP just means the browser declines to store the cookie.
fn is_secure_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| p.eq_ignore_ascii_case("https"))
}

/// Build the Set-Cookie string. Over HTTPS use the hardened `__Host-` +
/// `Secure` form; over plain HTTP drop both (the prefix requires `Secure`).
/// `SameSite=Lax` (was Strict) blocks cross-site sub-request CSRF while
/// allowing top-level navigation; the custom-header CSRF check (item 5)
/// covers the rest.
fn build_session_cookie(value: &str, max_age: u64, secure: bool) -> String {
    if secure {
        format!(
            "{SESSION_COOKIE_HOST}={value}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age}"
        )
    } else {
        format!("{SESSION_COOKIE}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}")
    }
}

#[derive(Debug, Deserialize)]
struct LoginPayload {
    api_key: String,
}

/// `POST /api/v1/auth/login` — body `{"api_key": "<key>"}`. On success
/// sets the `strivo_session` cookie + returns the expiry timestamp.
async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req_headers: HeaderMap,
    Json(body): Json<LoginPayload>,
) -> impl IntoResponse {
    let ip = peer.ip();
    if let crate::ratelimit::Decision::Blocked { retry_after_secs } = state.login_limiter.check(ip) {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from(retry_after_secs));
        return (
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            Json(json!({"error": "too many failed attempts; try again later"})),
        )
            .into_response();
    }

    if !state.api_key.matches(&body.api_key) {
        state.login_limiter.record_failure(ip);
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid api_key"})),
        )
            .into_response();
    }
    state.login_limiter.record_success(ip);

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
    let cookie = build_session_cookie(&cookie_value, SESSION_TTL_SECS, is_secure_request(&req_headers));

    let cookie_header = match HeaderValue::from_str(&cookie) {
        Ok(h) => h,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "could not encode session cookie"})),
            )
                .into_response();
        }
    };
    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, cookie_header);
    (
        StatusCode::OK,
        headers,
        Json(json!({"status": "ok", "expires_at": token.expires_at})),
    )
        .into_response()
}

/// `POST /api/v1/auth/logout` — clears the cookie. We don't know which name
/// the browser holds (depends whether login happened over HTTPS), so clear
/// both: the plain name and the `__Host-` name (the latter with `Secure`, as
/// the prefix requires). Each is appended as its own Set-Cookie header.
async fn logout() -> impl IntoResponse {
    let clears = [
        format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0"),
        format!("{SESSION_COOKIE_HOST}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"),
    ];
    let mut headers = HeaderMap::new();
    for c in &clears {
        // Fixed strings; skip rather than panic if one fails to parse.
        if let Ok(h) = HeaderValue::from_str(c) {
            headers.append(SET_COOKIE, h);
        }
    }
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
    // Accept either the plain (HTTP) or the `__Host-` (HTTPS) name.
    for pair in cookie_header.split(';').map(|s| s.trim()) {
        let value = pair
            .strip_prefix(&format!("{SESSION_COOKIE_HOST}="))
            .or_else(|| pair.strip_prefix(&format!("{SESSION_COOKIE}=")));
        if let Some(value) = value {
            if let Some(tok) = SessionToken::decode_verify(value, secret) {
                return Some(tok);
            }
        }
    }
    None
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

/// Idle-refresh policy: re-issue once the cookie is past the halfway point
/// of its lifetime, giving a rolling expiry that slides forward on activity
/// (roadmap item 4). Returns false for tokens with plenty of life left so we
/// don't write a Set-Cookie on every request.
fn should_refresh(expires_at: u64, now: u64, ttl: u64) -> bool {
    expires_at.saturating_sub(now) < ttl / 2
}

/// Middleware: when a request authenticated by a valid session cookie is
/// past the halfway mark, append a freshly-signed cookie so active sessions
/// never expire mid-use. No-op for X-Api-Key clients (no cookie), for the
/// auth endpoints themselves (login sets / logout clears the cookie), and
/// for tokens still in the first half of their life.
pub async fn session_refresh(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path().to_string();
    let req_headers = req.headers().clone();
    let mut resp = next.run(req).await;

    if path.starts_with("/api/v1/auth/") {
        return resp;
    }
    let Some(secret) = state.session_secret.as_deref() else {
        return resp;
    };
    let Some(tok) = session_from_headers(&req_headers, Some(secret)) else {
        return resp;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if !should_refresh(tok.expires_at, now, SESSION_TTL_SECS) {
        return resp;
    }
    let value = SessionToken::new(SESSION_TTL_SECS).encode(secret);
    let cookie = build_session_cookie(&value, SESSION_TTL_SECS, is_secure_request(&req_headers));
    if let Ok(h) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().append(SET_COOKIE, h);
    }
    resp
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{generate_session_secret, SessionToken};

    #[test]
    fn secure_request_uses_host_prefixed_secure_cookie() {
        let c = build_session_cookie("abc.def", 60, true);
        assert!(c.starts_with(&format!("{SESSION_COOKIE_HOST}=abc.def")));
        assert!(c.contains("; Secure"));
        assert!(c.contains("; SameSite=Lax"));
        assert!(c.contains("; Path=/"));
    }

    #[test]
    fn insecure_request_uses_plain_cookie_without_secure() {
        let c = build_session_cookie("abc.def", 60, false);
        assert!(c.starts_with(&format!("{SESSION_COOKIE}=abc.def")));
        assert!(!c.contains("Secure"));
        assert!(!c.contains("__Host-"));
        assert!(c.contains("; SameSite=Lax"));
    }

    #[test]
    fn is_secure_request_reads_forwarded_proto() {
        let mut h = HeaderMap::new();
        assert!(!is_secure_request(&h));
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(is_secure_request(&h));
        h.insert("x-forwarded-proto", HeaderValue::from_static("http"));
        assert!(!is_secure_request(&h));
    }

    #[test]
    fn session_read_accepts_both_cookie_names() {
        let secret = generate_session_secret();
        let value = SessionToken::new(60).encode(&secret);

        for name in [SESSION_COOKIE, SESSION_COOKIE_HOST] {
            let mut h = HeaderMap::new();
            h.insert(
                "cookie",
                HeaderValue::from_str(&format!("foo=bar; {name}={value}")).unwrap(),
            );
            assert!(
                session_from_headers(&h, Some(&secret)).is_some(),
                "cookie name {name} should verify"
            );
        }
    }

    #[test]
    fn refresh_only_past_halfway() {
        let ttl = 1000;
        let now = 10_000;
        // Fresh token (full life left) → no refresh.
        assert!(!should_refresh(now + ttl, now, ttl));
        // Exactly at halfway boundary → not yet (strict <).
        assert!(!should_refresh(now + ttl / 2, now, ttl));
        // Past halfway → refresh.
        assert!(should_refresh(now + ttl / 2 - 1, now, ttl));
        // Nearly expired → refresh.
        assert!(should_refresh(now + 1, now, ttl));
        // Already expired → refresh (saturating_sub → 0).
        assert!(should_refresh(now.saturating_sub(5), now, ttl));
    }

    #[test]
    fn session_read_rejects_bad_value_under_valid_name() {
        let secret = generate_session_secret();
        let mut h = HeaderMap::new();
        h.insert(
            "cookie",
            HeaderValue::from_str(&format!("{SESSION_COOKIE}=not-a-token")).unwrap(),
        );
        assert!(session_from_headers(&h, Some(&secret)).is_none());
    }

    // --- check_dual: the integrated auth gate (HMAC verify + header path) ---

    #[test]
    fn check_dual_accepts_valid_api_key() {
        let key = crate::auth::ApiKey("secret-key".into());
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("secret-key"));
        assert!(check_dual(&h, &key, None).is_ok());
    }

    #[test]
    fn check_dual_rejects_wrong_api_key() {
        let key = crate::auth::ApiKey("secret-key".into());
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("nope"));
        assert_eq!(check_dual(&h, &key, None).unwrap_err(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn check_dual_accepts_valid_cookie() {
        let key = crate::auth::ApiKey("secret-key".into());
        let secret = generate_session_secret();
        let value = SessionToken::new(60).encode(&secret);
        let mut h = HeaderMap::new();
        h.insert(
            "cookie",
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={value}")).unwrap(),
        );
        // Wrong/absent api-key, but the cookie alone authorizes.
        assert!(check_dual(&h, &key, Some(&secret)).is_ok());
    }

    #[test]
    fn check_dual_rejects_when_neither_present() {
        let key = crate::auth::ApiKey("secret-key".into());
        let secret = generate_session_secret();
        let h = HeaderMap::new();
        assert_eq!(
            check_dual(&h, &key, Some(&secret)).unwrap_err(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn check_dual_rejects_cookie_signed_by_other_secret() {
        let key = crate::auth::ApiKey("secret-key".into());
        let real = generate_session_secret();
        let attacker = generate_session_secret();
        let value = SessionToken::new(60).encode(&attacker);
        let mut h = HeaderMap::new();
        h.insert(
            "cookie",
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={value}")).unwrap(),
        );
        assert_eq!(
            check_dual(&h, &key, Some(&real)).unwrap_err(),
            StatusCode::UNAUTHORIZED
        );
    }
}
