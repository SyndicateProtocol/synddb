//! Gas funding relayer for `SyndDB` TEE keys
//!
//! This service handles key registration and funding for TEE keys that
//! don't have gas to submit their own transactions.

mod config;
mod handlers;
mod submitter;
mod tracker;

use crate::{
    config::RelayerConfig,
    handlers::{health, register_and_fund},
    submitter::RelayerSubmitter,
    tracker::FundingTracker,
};
use axum::{
    routing::{get, post},
    Extension, Router,
};
use clap::Parser;
use std::sync::Arc;
use tokio::sync::RwLock;
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

    // Parse configuration
    let config = RelayerConfig::parse();
    config.validate()?;

    info!(
        listen_addr = %config.listen_addr,
        treasury_address = %config.treasury_address,
        key_manager_address = %config.key_manager_address,
        "Starting relayer"
    );

    // Initialize components
    let submitter = Arc::new(RelayerSubmitter::from_config(&config)?);
    let tracker = Arc::new(RwLock::new(FundingTracker::new(&config)));

    // Build router
    let app = Router::new()
        .route("/health", get(health))
        .route("/register-and-fund", post(register_and_fund))
        .layer(Extension(config.clone()))
        .layer(Extension(submitter))
        .layer(Extension(tracker));

    // Start server
    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    info!(addr = %config.listen_addr, "Relayer listening");

    axum::serve(listener, app).await?;

    Ok(())
}
