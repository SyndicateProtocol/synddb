//! `SyndDB` Sequencer - Changeset Receiver and Publisher
//!
//! The sequencer receives changesets from synddb-client libraries via HTTP
//! and publishes them to multiple DA layers.

pub mod attestor;
pub mod batch;
pub mod config;
pub mod messages;
pub mod monitor;
pub mod publish;
pub mod tee;
pub mod utils;
pub mod http_api;

pub use config::Config;

use anyhow::Result;
use tracing::info;

/// Main sequencer runtime
#[derive(Debug)]
pub struct Sequencer {
    #[allow(dead_code)] // Will be used when implementation is added
    config: Config,
}

impl Sequencer {
    pub const fn new(config: Config) -> Self {
        Self { config }
    }

    pub async fn run(&self) -> Result<()> {
        info!("SyndDB Sequencer running...");

        // TODO: Initialize components:
        // 1. HTTP Receiver - receive changesets from client libraries
        // 2. Batcher - accumulate changesets
        // 3. Attestor - compress and sign batches
        // 4. Publisher - publish to DA layers
        // 5. Message Monitor - inbound/outbound message handling

        Ok(())
    }
}
