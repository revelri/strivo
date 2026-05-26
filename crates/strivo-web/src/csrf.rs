//! CSRF mitigation for state-changing requests.
//!
//! The web UI binds to localhost by default and acts as a thin proxy over
//! the daemon IPC. Per W1 in the code audit we don't issue per-form
//! tokens (yet) — instead we rely on the standard Origin / Referer +
//! Host-match check, which defeats the cross-origin form-submit class
//! of CSRF without requiring every template to embed a hidden field.
//!
//! Two auth tracks, two CSRF stances (only on POST/PUT/PATCH/DELETE):
//!
//! - **Programmatic track** — any request carrying `X-Api-Key`. A browser
//!   can't attach a custom header cross-site without a CORS preflight (which
//!   we never grant), so this track is CSRF-immune by construction. Allowed
//!   through; the key's *validity* is checked downstream by `check_key`.
//! - **Cookie / browser track** — everything else (the SPA authenticates
//!   via the session cookie). Required to carry a custom `X-Strivo-CSRF`
//!   (or `X-Requested-With`) header AND have its `Origin`/`Referer` match
//!   the `Host`. The custom header alone defeats classic CSRF; the
//!   Origin/Host allowlist is defense in depth.
//!
//! GET / HEAD / OPTIONS are safe by definition and skip the check.
//!
//! Users binding the web UI to a non-loopback interface should still front
//! it with a reverse proxy (e.g. `tailscale serve`) terminating TLS.

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

const API_KEY_HEADER: &str = "x-api-key";
const CSRF_HEADER: &str = "x-strivo-csrf";

pub async fn csrf_guard(req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    if !is_state_changing(req.method()) {
        return Ok(next.run(req).await);
    }

    let headers = req.headers();

    // Programmatic track: presence of the custom X-Api-Key header marks a
    // non-browser caller (can't be forged cross-site without a denied
    // preflight). Validity is enforced by the handler's check_key.
    if headers.contains_key(API_KEY_HEADER) {
        return Ok(next.run(req).await);
    }

    // Cookie/browser track: custom CSRF header AND Origin/Host match.
    if has_csrf_header(headers) && origin_matches_host(headers) {
        Ok(next.run(req).await)
    } else {
        tracing::warn!(
            method = %req.method(),
            path = req.uri().path(),
            "csrf: blocked cookie-track mutation missing X-Strivo-CSRF header or Origin/Host match"
        );
        Err(StatusCode::FORBIDDEN)
    }
}

fn is_state_changing(method: &Method) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::PATCH | Method::DELETE)
}

/// A browser can only set these custom headers on a same-origin request
/// (cross-site fetch would need a CORS preflight we never grant).
fn has_csrf_header(headers: &HeaderMap) -> bool {
    headers.contains_key(CSRF_HEADER) || headers.contains_key("x-requested-with")
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

    #[test]
    fn csrf_header_detected() {
        let mut h = HeaderMap::new();
        assert!(!has_csrf_header(&h));
        h.insert(CSRF_HEADER, HeaderValue::from_static("1"));
        assert!(has_csrf_header(&h));

        let mut h2 = HeaderMap::new();
        h2.insert("x-requested-with", HeaderValue::from_static("XMLHttpRequest"));
        assert!(has_csrf_header(&h2));
    }

    #[test]
    fn cookie_track_needs_both_header_and_origin() {
        // Header present but no Origin → not enough.
        let mut only_header = hm("127.0.0.1:8181", None, None);
        only_header.insert(CSRF_HEADER, HeaderValue::from_static("1"));
        assert!(has_csrf_header(&only_header));
        assert!(!origin_matches_host(&only_header));

        // Origin present but no custom header → not enough.
        let only_origin = hm("127.0.0.1:8181", Some("http://127.0.0.1:8181"), None);
        assert!(!has_csrf_header(&only_origin));
        assert!(origin_matches_host(&only_origin));

        // Both present and matching → the only accepted cookie-track shape.
        let mut both = hm("127.0.0.1:8181", Some("http://127.0.0.1:8181"), None);
        both.insert(CSRF_HEADER, HeaderValue::from_static("1"));
        assert!(has_csrf_header(&both) && origin_matches_host(&both));
    }
}
