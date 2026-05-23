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
    // Read session_secret from config if present; W3's /login will
    // generate one lazily on first sign-in if it's absent.
    let session_secret = strivo_core::config::AppConfig::load(None)
        .ok()
        .and_then(|c| c.web.session_secret.clone());
    let state = AppState {
        ipc,
        api_key: cfg.api_key,
        session_secret,
    };

    let app = Router::new()
        .merge(routes::dashboard::router())
        .merge(routes::channels::router())
        .merge(routes::recordings::router())
        .merge(routes::schedule::router())
        .merge(routes::settings::router())
        .merge(routes::logs::router())
        .merge(routes::system::router())
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
