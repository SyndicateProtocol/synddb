//! HTTP API for bidirectional messaging

use super::InboundMessage;
use anyhow::Result;
use axum::{routing::post, Router};
use std::net::SocketAddr;
use tokio::sync::mpsc::Sender;

pub struct MessageApi {
    port: u16,
    _inbound_tx: Sender<InboundMessage>,
}

impl MessageApi {
    pub fn new(port: u16, inbound_tx: Sender<InboundMessage>) -> Self {
        Self {
            port,
            _inbound_tx: inbound_tx,
        }
    }

    pub async fn serve(&self) -> Result<()> {
        // TODO: Create HTTP API with axum
        // POST /messages - Application can fetch inbound messages
        // GET /health - Health check

        let app = Router::new().route("/messages", post(handle_message_delivery));

        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        tracing::info!("Message API listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

async fn handle_message_delivery() -> &'static str {
    // TODO: Implement message delivery endpoint
    "OK"
}
