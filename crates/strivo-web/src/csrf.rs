//! CSRF mitigation for state-changing requests.
//!
//! The web UI binds to localhost by default and acts as a thin proxy over
//! the daemon IPC. Per W1 in the code audit we don't issue per-form
//! tokens (yet) — instead we rely on the standard Origin / Referer +
//! Host-match check, which defeats the cross-origin form-submit class
//! of CSRF without requiring every template to embed a hidden field.
//!
//! What we enforce (only on POST/PUT/PATCH/DELETE):
//!
//! 1. The request carries an `Origin` (preferred) or `Referer` header.
//! 2. Its scheme+host+port matches the server's `Host` header.
//!
//! What we deliberately do **not** check:
//!
//! - `/api/v1/*` — already protected by the constant-time `X-Api-Key`
//!   header; CORS-preflighted browsers can't forge that header.
//! - GET / HEAD / OPTIONS — safe by definition.
//!
//! This is documented in `crates/strivo-web/README.md`. Users binding
//! the web UI to a non-loopback interface should run it behind a
//! reverse proxy that adds CSRF tokens (e.g. nginx + a session cookie
//! middleware) until a full per-form token scheme lands.

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

pub async fn csrf_guard(req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    if !is_state_changing(req.method()) {
        return Ok(next.run(req).await);
    }

    // /api/v1 is authenticated separately via X-Api-Key + constant-time
    // compare. CSRF doesn't apply there: a browser cross-origin form
    // can't add custom headers without a preflight, and the preflight
    // would be denied (no permissive CORS handler).
    let path = req.uri().path();
    if path.starts_with("/api/v1") {
        return Ok(next.run(req).await);
    }

    let headers = req.headers();
    if origin_matches_host(headers) {
        Ok(next.run(req).await)
    } else {
        tracing::warn!(
            method = %req.method(),
            path,
            "csrf: rejecting state-changing request with missing or mismatched Origin/Referer"
        );
        Err(StatusCode::FORBIDDEN)
    }
}

fn is_state_changing(method: &Method) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::PATCH | Method::DELETE)
}

fn origin_matches_host(headers: &HeaderMap) -> bool {
    let host = match headers.get("Host").and_then(|h| h.to_str().ok()) {
        Some(h) => h,
        None => return false,
    };

    // Origin is the preferred signal (modern browsers send it on
    // POST). Fall back to Referer for older clients / curl scripts
    // that only set Referer.
    let candidate = headers
        .get("Origin")
        .and_then(|h| h.to_str().ok())
        .or_else(|| headers.get("Referer").and_then(|h| h.to_str().ok()));

    let Some(value) = candidate else { return false };

    // Extract host[:port] from "scheme://host[:port]/...".
    let stripped = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .unwrap_or(value);
    let value_host = stripped.split('/').next().unwrap_or("");

    value_host.eq_ignore_ascii_case(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn hm(host: &str, origin: Option<&str>, referer: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("Host", HeaderValue::from_str(host).unwrap());
        if let Some(o) = origin {
            h.insert("Origin", HeaderValue::from_str(o).unwrap());
        }
        if let Some(r) = referer {
            h.insert("Referer", HeaderValue::from_str(r).unwrap());
        }
        h
    }

    #[test]
    fn origin_match_accepts() {
        let h = hm("127.0.0.1:8181", Some("http://127.0.0.1:8181"), None);
        assert!(origin_matches_host(&h));
    }

    #[test]
    fn cross_origin_rejected() {
        let h = hm("127.0.0.1:8181", Some("http://evil.example.com"), None);
        assert!(!origin_matches_host(&h));
    }

    #[test]
    fn missing_origin_and_referer_rejected() {
        let h = hm("127.0.0.1:8181", None, None);
        assert!(!origin_matches_host(&h));
    }

    #[test]
    fn referer_used_when_origin_missing() {
        let h = hm(
            "127.0.0.1:8181",
            None,
            Some("http://127.0.0.1:8181/settings"),
        );
        assert!(origin_matches_host(&h));
    }

    #[test]
    fn https_scheme_accepted_when_host_matches() {
        let h = hm("strivo.local", Some("https://strivo.local"), None);
        assert!(origin_matches_host(&h));
    }

    #[test]
    fn safe_methods_skip_check() {
        assert!(!is_state_changing(&Method::GET));
        assert!(!is_state_changing(&Method::HEAD));
        assert!(!is_state_changing(&Method::OPTIONS));
        assert!(is_state_changing(&Method::POST));
        assert!(is_state_changing(&Method::DELETE));
    }
}
