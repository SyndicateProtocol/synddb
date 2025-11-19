//! Batching logic for changesets and snapshots

pub mod accumulator;
pub mod snapshot;
pub mod timer;

pub use accumulator::Batcher;
pub use snapshot::SnapshotCreator;

use crate::monitor::{Changeset, SchemaChange};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Batch payload types for publishing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BatchPayload {
    /// Full database snapshot triggered by schema change
    /// Includes DDL statements for audit trail
    SnapshotWithSchemaChange {
        snapshot_data: Vec<u8>,
        schema_change: SchemaChange,
        sequence: u64,
        timestamp: SystemTime,
    },

    /// Incremental changesets (deterministic replay)
    ChangesetBatch {
        changesets: Vec<Changeset>,
        sequence: u64,
        timestamp: SystemTime,
    },

    /// Full database snapshot (periodic recovery point)
    Snapshot {
        data: Vec<u8>,
        sequence: u64,
        timestamp: SystemTime,
    },
}
