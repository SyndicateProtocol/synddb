//! SQLite Session Extension integration

use anyhow::Result;
use crossbeam_channel::Sender;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use tracing::{debug, trace};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    /// Raw changeset bytes from SQLite
    pub data: Vec<u8>,
    /// Sequence number (monotonic)
    pub sequence: u64,
    /// Timestamp when captured
    pub timestamp: SystemTime,
}

pub struct SessionMonitor {
    changeset_tx: Sender<Changeset>,
    sequence: u64,
}

impl SessionMonitor {
    pub fn new(_conn: &Connection, changeset_tx: Sender<Changeset>) -> Result<Self> {
        // TODO: Initialize SQLite Session Extension
        // This requires using rusqlite's session feature

        debug!("SessionMonitor created");

        Ok(Self {
            changeset_tx,
            sequence: 0,
        })
    }

    pub fn start(&self) -> Result<()> {
        // TODO: Register commit hook on the connection
        // The hook will call self.on_commit() after each successful commit

        debug!("SessionMonitor started");
        Ok(())
    }

    fn on_commit(&mut self, changeset_data: Vec<u8>) -> Result<()> {
        trace!("Captured changeset: {} bytes", changeset_data.len());

        let changeset = Changeset {
            data: changeset_data,
            sequence: self.sequence,
            timestamp: SystemTime::now(),
        };

        self.sequence += 1;

        // Send to background thread
        self.changeset_tx.send(changeset)?;

        Ok(())
    }
}
