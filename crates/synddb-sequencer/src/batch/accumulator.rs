//! Accumulate changesets and trigger publishing

use super::BatchPayload;
use crate::config::BatchConfig;
use crate::monitor::{Changeset, SchemaChange};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::mpsc::{Receiver, Sender};

pub struct Batcher {
    db_path: PathBuf,
    changesets: Vec<Changeset>,
    buffer_size: usize,
    last_flush: Instant,
    last_snapshot: Instant,
    batch_count: usize,
    sequence: AtomicU64,
    config: BatchConfig,
}

impl Batcher {
    pub fn new(db_path: PathBuf, config: BatchConfig) -> Self {
        Self {
            db_path,
            changesets: Vec::new(),
            buffer_size: 0,
            last_flush: Instant::now(),
            last_snapshot: Instant::now(),
            batch_count: 0,
            sequence: AtomicU64::new(0),
            config,
        }
    }

    pub async fn run(
        mut self,
        mut changeset_rx: Receiver<Changeset>,
        mut schema_rx: Receiver<SchemaChange>,
        tx: Sender<BatchPayload>,
    ) -> Result<()> {
        // TODO: Implement batching loop
        // 1. Receive changesets and schema changes
        // 2. Accumulate changesets until threshold
        // 3. Create snapshots on schema change or timer
        // 4. Send batches to publisher

        Ok(())
    }

    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst)
    }

    async fn create_snapshot(&self) -> Result<Vec<u8>> {
        // TODO: Create full database snapshot
        Ok(vec![])
    }

    fn should_flush(&self) -> bool {
        self.buffer_size >= self.config.max_batch_size
            || self.last_flush.elapsed() >= self.config.max_batch_age
    }

    fn should_snapshot(&self) -> bool {
        self.batch_count >= self.config.snapshot_threshold
            || self.last_snapshot.elapsed() >= self.config.snapshot_interval
    }
}
