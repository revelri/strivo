//! Strivo Pro licence status endpoint (Phase 1 stub).
//!
//! Returns the current entitlement so the SPA can decide whether to show
//! the upgrade card. The real implementation lives behind the activation
//! backend (CF Workers + D1) and the in-app licence cache (Phase 3).
//! Until then this returns a hard-coded "free, not entitled" payload so
//! the UI surface lights up.

use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::json;

use crate::server::AppState;

#[derive(Serialize)]
struct LicenceStatus {
    /// True if any Pro feature should unlock.
    entitled: bool,
    /// "free" | "pro" | "trial".
    tier: &'static str,
    /// Present when a trial is active; null otherwise.
    trial: Option<serde_json::Value>,
    /// ISO-8601 expiry for paid/trial; null for free.
    expires_at: Option<String>,
    /// Set once Phase 3 is wired up.
    machine_id: Option<String>,
    /// Tells the SPA the real backend is offline — show the upgrade card
    /// but disable the "Activate" button (the trial CTA stays live).
    implemented: bool,
}

async fn status() -> Json<LicenceStatus> {
    use strivo_core::licence::{gate, machine_id, LicenceCache, Tier};

    // Surface the hashed machine_id so the activation flow can pin it
    // without ever sending the raw OS-level identifier. Read once per
    // request; the underlying call memoises.
    let mh = machine_id::hashed_machine_id();

    // gate::entitled handles the dev override + cache check in one
    // place so the UI and the runtime gate stay in lockstep.
    let entitled = gate::entitled();
    let cache = LicenceCache::load().ok().flatten();

    let (tier, expires_at, trial) = match cache.as_ref() {
        Some(lic) if entitled => {
            let tier = match lic.tier {
                Tier::Pro => "pro",
                Tier::Trial => "trial",
                Tier::Free => "free",
            };
            let trial = if matches!(lic.tier, Tier::Trial) {
                Some(serde_json::json!({ "expires_at": lic.expires_at }))
            } else {
                None
            };
            (tier, lic.expires_at.clone(), trial)
        }
        _ if entitled => ("pro", None, None), // dev override w/o cache
        _ => ("free", None, None),
    };

    Json(LicenceStatus {
        entitled,
        tier,
        trial,
        expires_at,
        machine_id: Some(mh),
        // Real reads now wired; mutating routes (activate/trial/refresh)
        // still 501 until the CF Workers backend (Phase 3b) is up.
        implemented: false,
    })
}

async fn not_implemented() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "not_implemented",
            "message": "Activation backend is not wired yet — coming in Phase 3."
        })),
    )
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/licence/status", get(status))
        .route(
            "/api/v1/licence/activate",
            axum::routing::post(not_implemented),
        )
        .route(
            "/api/v1/licence/trial",
            axum::routing::post(not_implemented),
        )
        .route(
            "/api/v1/licence/refresh",
            axum::routing::post(not_implemented),
        )
}
