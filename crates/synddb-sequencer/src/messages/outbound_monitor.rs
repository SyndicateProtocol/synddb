//! `SQLite` table monitoring for outbound messages

use super::OutboundMessage;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc::Sender;

#[derive(Debug)]
pub struct OutboundMonitor {
    _db_path: PathBuf,
    _message_tx: Sender<OutboundMessage>,
}

impl OutboundMonitor {
    pub const fn new(db_path: PathBuf, message_tx: Sender<OutboundMessage>) -> Self {
        Self {
            _db_path: db_path,
            _message_tx: message_tx,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // TODO: Monitor message tables in SQLite
        // TODO: Detect new outbound messages
        // TODO: Send to channel for processing

        Ok(())
    }
}
