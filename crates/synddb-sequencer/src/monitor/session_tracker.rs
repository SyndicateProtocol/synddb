//! SQLite Session Extension wrapper for changeset tracking

use super::{Changeset, SchemaChange};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc::Sender;

pub struct SessionMonitor {
    _db_path: PathBuf,
    _changeset_tx: Sender<Changeset>,
    _schema_tx: Sender<SchemaChange>,
    _last_schema_version: i32,
}

impl SessionMonitor {
    pub async fn new(
        db_path: PathBuf,
        changeset_tx: Sender<Changeset>,
        schema_tx: Sender<SchemaChange>,
    ) -> Result<Self> {
        // TODO: Initialize SQLite connection with Session Extension
        // TODO: Set up update hooks for changeset capture
        // TODO: Monitor sqlite_schema table for DDL changes

        Ok(Self {
            _db_path: db_path,
            _changeset_tx: changeset_tx,
            _schema_tx: schema_tx,
            _last_schema_version: 0,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // TODO: Main monitoring loop
        // 1. Attach Session Extension to database
        // 2. Register commit hook to capture changesets
        // 3. Monitor sqlite_schema for DDL operations
        // 4. Send changesets and schema changes to channels

        Ok(())
    }
}
