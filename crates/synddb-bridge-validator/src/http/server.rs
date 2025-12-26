use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tokio::net::TcpListener;
use tracing::info;

use super::{
    auth::require_api_key,
    handlers::{
        get_message_status, get_schema, health, ready, reject_message, reject_proposal,
        submit_message, AppState,
    },
};

pub async fn start_server(state: Arc<AppState>, host: &str, port: u16) -> Result<()> {
    // Protected routes (require API key if configured)
    let protected = Router::new()
        .route("/messages", post(submit_message))
        .route("/reject/proposal", post(reject_proposal))
        .route("/reject/message", post(reject_message))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    // Public routes (no authentication required)
    let public = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/messages/:id", get(get_message_status))
        .route("/schemas/:message_type", get(get_schema));

    let app = Router::new()
        .merge(protected)
        .merge(public)
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    let listener = TcpListener::bind(addr).await?;

    info!(address = %addr, "HTTP server listening");

    axum::serve(listener, app).await?;

    Ok(())
}
