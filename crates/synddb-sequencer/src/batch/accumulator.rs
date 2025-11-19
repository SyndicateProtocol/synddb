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
    _db_path: PathBuf,
    _changesets: Vec<Changeset>,
    _buffer_size: usize,
    _last_flush: Instant,
    _last_snapshot: Instant,
    _batch_count: usize,
    _sequence: AtomicU64,
    _config: BatchConfig,
}

impl Batcher {
    pub fn new(db_path: PathBuf, config: BatchConfig) -> Self {
        Self {
            _db_path: db_path,
            _changesets: Vec::new(),
            _buffer_size: 0,
            _last_flush: Instant::now(),
            _last_snapshot: Instant::now(),
            _batch_count: 0,
            _sequence: AtomicU64::new(0),
            _config: config,
        }
    }

    pub async fn run(
        self,
        _changeset_rx: Receiver<Changeset>,
        _schema_rx: Receiver<SchemaChange>,
        _tx: Sender<BatchPayload>,
    ) -> Result<()> {
        // TODO: Implement batching loop
        // 1. Receive changesets and schema changes
        // 2. Accumulate changesets until threshold
        // 3. Create snapshots on schema change or timer
        // 4. Send batches to publisher

        Ok(())
    }

    #[allow(dead_code)]
    fn next_sequence(&self) -> u64 {
        self._sequence.fetch_add(1, Ordering::SeqCst)
    }

    #[allow(dead_code)]
    async fn create_snapshot(&self) -> Result<Vec<u8>> {
        // TODO: Create full database snapshot
        Ok(vec![])
    }

    #[allow(dead_code)]
    fn should_flush(&self) -> bool {
        self._buffer_size >= self._config.max_batch_size
            || self._last_flush.elapsed() >= self._config.max_batch_age
    }

    #[allow(dead_code)]
    fn should_snapshot(&self) -> bool {
        self._batch_count >= self._config.snapshot_threshold
            || self._last_snapshot.elapsed() >= self._config.snapshot_interval
    }
}
