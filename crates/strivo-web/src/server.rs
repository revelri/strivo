use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{middleware, Router};
use tower_http::trace::TraceLayer;

use crate::auth::ApiKey;
use crate::csrf;
use crate::ipc_client::IpcClient;
use crate::routes;

#[derive(Clone)]
pub struct AppState {
    pub ipc: Arc<IpcClient>,
    pub api_key: ApiKey,
    /// HMAC secret for browser-session cookies (W3). Loaded from
    /// `WebConfig.session_secret`; `None` until the first /login
    /// generates and persists one.
    pub session_secret: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub bind: SocketAddr,
    pub api_key: ApiKey,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8181".parse().expect("hardcoded addr parses"),
            api_key: ApiKey::generate(),
        }
    }
}

pub async fn serve(cfg: ServeConfig) -> Result<()> {
    let ipc = Arc::new(IpcClient::connect_or_err()?);
    // Session secret must exist before the first request so the cookie
    // /login signs is verifiable by check_key on the same process. Read
    // from config, else generate + persist now (don't defer to /login —
    // that left AppState's copy out of sync with the signed cookie).
    let session_secret = {
        let mut cfg_file = strivo_core::config::AppConfig::load(None).ok();
        let existing = cfg_file.as_ref().and_then(|c| c.web.session_secret.clone());
        match existing {
            Some(s) => s,
            None => {
                let s = crate::auth::generate_session_secret();
                if let Some(ref mut c) = cfg_file {
                    c.web.session_secret = Some(s.clone());
                    if let Err(e) = c.save(None) {
                        tracing::warn!("could not persist [web].session_secret: {e}");
                    }
                }
                s
            }
        }
    };
    let state = AppState {
        ipc,
        api_key: cfg.api_key,
        session_secret: Some(session_secret),
    };

    // The SPA (served by assets::router at / and /app) is the webui; it
    // talks to the daemon exclusively through the JSON api + events + auth
    // routers. The legacy askama/htmx page routers (dashboard, channels,
    // recordings, schedule, settings, logs, system) are retired — they
    // served the old server-rendered UI at /, /channels, … and were the
    // reason the bare root showed the pre-redesign dashboard.
    let app = Router::new()
        .merge(routes::events::router())
        .merge(routes::api::router())
        .merge(routes::login::router())
        .merge(routes::assets::router())
        .layer(middleware::from_fn(csrf::csrf_guard))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;

    tracing::info!(addr = %cfg.bind, "strivo-web listening");
    axum::serve(listener, app).await?;
    Ok(())
}
