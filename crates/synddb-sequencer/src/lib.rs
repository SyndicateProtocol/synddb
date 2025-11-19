//! SyndDB Sequencer - Lightweight SQLite Monitor and Publisher
//!
//! The sequencer runs as a sidecar process that attaches to any SQLite database
//! using the SQLite Session Extension to capture deterministic changesets.
//! It publishes logical database changes to multiple DA layers.

pub mod attestor;
pub mod batch;
pub mod config;
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
        info!("Database: {:?}", self.config.database.path);

        // TODO: Initialize components:
        // 1. Session Monitor - attach to SQLite via Session Extension
        // 2. Batcher - accumulate changesets
        // 3. Attestor - compress and sign batches
        // 4. Publisher - publish to DA layers
        // 5. Message Monitor - inbound/outbound message handling

        Ok(())
    }
}
