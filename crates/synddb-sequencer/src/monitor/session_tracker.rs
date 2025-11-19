//! SQLite Session Extension wrapper for changeset tracking
//!
//! **Note**: This module is not currently used by the sequencer service.
//! Session monitoring is handled by the synddb-client library which embeds
//! in applications. This code is kept for reference and potential future use.

use super::{Changeset, SchemaChange};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc::Sender;

#[allow(dead_code)]
pub struct SessionMonitor {
    db_path: PathBuf,
    changeset_tx: Sender<Changeset>,
    schema_tx: Sender<SchemaChange>,
    last_schema_version: i32,
}

impl SessionMonitor {
    #[allow(dead_code)]
    pub async fn new(
        db_path: PathBuf,
        changeset_tx: Sender<Changeset>,
        schema_tx: Sender<SchemaChange>,
    ) -> Result<Self> {
        // NOTE: Session monitoring is now handled by synddb-client library
        // This implementation is kept for reference only

        Ok(Self {
            db_path,
            changeset_tx,
            schema_tx,
            last_schema_version: 0,
        })
    }

    #[allow(dead_code)]
    pub async fn run(&mut self) -> Result<()> {
        // NOTE: Session monitoring is now handled by synddb-client library
        // The sequencer receives changesets via HTTP instead

        Ok(())
    }
}
