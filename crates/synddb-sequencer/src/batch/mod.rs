//! Batching logic for changesets and snapshots received from clients

pub mod accumulator;
pub mod timer;

pub use accumulator::Batcher;

use crate::monitor::{Changeset, SchemaChange};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Batch payload types received from clients for publishing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BatchPayload {
    /// Full database snapshot with schema change (received from client)
    /// Includes DDL statements for audit trail
    SnapshotWithSchemaChange {
        snapshot_data: Vec<u8>,
        schema_change: SchemaChange,
        sequence: u64,
        timestamp: SystemTime,
    },

    /// Incremental changesets (received from client)
    ChangesetBatch {
        changesets: Vec<Changeset>,
        sequence: u64,
        timestamp: SystemTime,
    },

    /// Full database snapshot (periodic recovery point from client)
    Snapshot {
        data: Vec<u8>,
        sequence: u64,
        timestamp: SystemTime,
    },
}
