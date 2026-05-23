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
    let state = AppState {
        ipc,
        api_key: cfg.api_key,
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
