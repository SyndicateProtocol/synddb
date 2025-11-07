//! SQLite table monitoring for outbound messages

use super::OutboundMessage;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc::Sender;

pub struct OutboundMonitor {
    db_path: PathBuf,
    message_tx: Sender<OutboundMessage>,
}

impl OutboundMonitor {
    pub fn new(db_path: PathBuf, message_tx: Sender<OutboundMessage>) -> Self {
        Self {
            db_path,
            message_tx,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // TODO: Monitor message tables in SQLite
        // TODO: Detect new outbound messages
        // TODO: Send to channel for processing

        Ok(())
    }
}
