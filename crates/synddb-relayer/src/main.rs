// Binary-only crate: pub(crate) is semantically correct but triggers redundant_pub_crate
// since modules are private. Allow this lint for binary crates.
#![allow(clippy::redundant_pub_crate)]

//! Gas relayer for `SyndDB` TEE key registration
//!
//! This service handles key registration for TEE keys that don't have gas
//! to submit their own transactions. The relayer pays gas on behalf of TEEs.
//!
//! Supports two configuration modes:
//! 1. Multi-tenant via TOML config file (set `RELAYER_CONFIG_PATH`)
//! 2. Single-application via environment variables (fallback)

mod config;
mod handlers;
mod submitter;

use crate::{
    config::RelayerConfig,
    handlers::{health, register_key},
    submitter::RelayerSubmitter,
};
use axum::{
    routing::{get, post},
    Extension, Router,
};
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(Level::INFO.into())
                .from_env_lossy(),
        )
        .init();

    // Load configuration
    let config = RelayerConfig::load()?;

    info!(
        listen_addr = %config.listen_addr,
        bridge_address = %config.bridge_address,
        application_count = config.applications.len(),
        "Starting relayer"
    );

    for (audience_hash, app) in &config.applications {
        info!(
            audience_hash = %audience_hash,
            audience = %app.audience,
            allowed_digests = app.allowed_image_digests.len(),
            "Configured application"
        );
    }

    // Initialize components
    let submitter = Arc::new(RelayerSubmitter::from_config(&config).await?);
    let config = Arc::new(config);

    // Build router
    let app = Router::new()
        .route("/health", get(health))
        .route("/register-key", post(register_key))
        .layer(Extension(config.clone()))
        .layer(Extension(submitter));

    // Start server
    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    info!(addr = %config.listen_addr, "Relayer listening");

    axum::serve(listener, app).await?;

    Ok(())
}
