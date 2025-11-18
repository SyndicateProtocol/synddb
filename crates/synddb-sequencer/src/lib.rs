//! SyndDB Sequencer - Receives changesets from application TEE
//!
//! The sequencer runs in a separate TEE from the application to isolate signing keys.
//! It receives changesets via HTTP from the application, validates them, signs them,
//! and publishes to multiple DA layers.

pub mod attestor;
pub mod batch;
pub mod config;
pub mod http_api;
pub mod messages;
pub mod monitor;
pub mod publish;
pub mod tee;
pub mod utils;

pub use config::Config;

use anyhow::Result;
use tracing::info;

/// Main sequencer runtime
pub struct Sequencer {
    config: Config,
}

impl Sequencer {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub async fn run(&self) -> Result<()> {
        info!("SyndDB Sequencer starting...");
        info!("Mode: Receive changesets via HTTP from application TEE");

        // TODO: Initialize components:
        // 1. HTTP API - receive changesets from application
        // 2. Batcher - accumulate changesets
        // 3. Attestor - compress and sign batches
        // 4. Publisher - publish to DA layers
        // 5. Message Monitor - inbound/outbound message handling

        // For now, just start HTTP server
        info!("Starting HTTP API on port 8433...");
        // TODO: Implement server startup

        Ok(())
    }
}
