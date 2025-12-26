use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::{
    routing::{get, post},
    Router,
};
use tokio::net::TcpListener;
use tracing::info;

use super::handlers::{get_message_status, get_schema, health, ready, submit_message, AppState};
use crate::config::ValidatorConfig;

pub async fn start_server(config: &ValidatorConfig) -> Result<()> {
    let state = Arc::new(AppState {
        mode: config.mode,
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/messages", post(submit_message))
        .route("/messages/:id", get(get_message_status))
        .route("/schemas/:message_type", get(get_schema))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.http_host, config.http_port).parse()?;
    let listener = TcpListener::bind(addr).await?;

    info!(address = %addr, "HTTP server listening");

    axum::serve(listener, app).await?;

    Ok(())
}
