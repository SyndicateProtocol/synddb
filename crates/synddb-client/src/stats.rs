//! Shared statistics for `SyndDB` replication status

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::RwLock;

/// Shared statistics about replication status
///
/// This struct is shared between the main `SyndDB` handle and background sender threads.
/// All fields use atomic or lock-protected types for thread-safe access.
#[derive(Debug)]
pub struct ReplicationStats {
    /// Number of changesets pending in the buffer
    pub(crate) pending_changesets: AtomicUsize,
    /// Number of changesets successfully published
    pub(crate) published_changesets: AtomicU64,
    /// Number of failed publish attempts
    pub(crate) failed_publishes: AtomicU64,
    /// Whether the last health check succeeded
    pub(crate) last_health_check_ok: AtomicBool,
    /// Time of last successful publish (protected by `RwLock` for `Instant`)
    pub(crate) last_publish_time: RwLock<Option<Instant>>,
    /// Time of last successful health check
    pub(crate) last_health_check_time: RwLock<Option<Instant>>,
}

impl Default for ReplicationStats {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplicationStats {
    /// Create new stats instance
    pub fn new() -> Self {
        Self {
            pending_changesets: AtomicUsize::new(0),
            published_changesets: AtomicU64::new(0),
            failed_publishes: AtomicU64::new(0),
            last_health_check_ok: AtomicBool::new(false),
            last_publish_time: RwLock::new(None),
            last_health_check_time: RwLock::new(None),
        }
    }

    /// Get number of changesets pending in buffer
    pub fn pending_count(&self) -> usize {
        self.pending_changesets.load(Ordering::Relaxed)
    }

    /// Get total number of successfully published changesets
    pub fn published_count(&self) -> u64 {
        self.published_changesets.load(Ordering::Relaxed)
    }

    /// Get number of failed publish attempts
    pub fn failed_count(&self) -> u64 {
        self.failed_publishes.load(Ordering::Relaxed)
    }

    /// Check if the last health check succeeded
    pub fn is_healthy(&self) -> bool {
        self.last_health_check_ok.load(Ordering::Relaxed)
    }

    /// Increment pending count
    pub(crate) fn increment_pending(&self) {
        self.pending_changesets.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement pending count by n
    pub(crate) fn decrement_pending(&self, n: usize) {
        self.pending_changesets.fetch_sub(n, Ordering::Relaxed);
    }

    /// Record a successful publish
    pub(crate) async fn record_publish(&self, count: usize) {
        self.published_changesets
            .fetch_add(count as u64, Ordering::Relaxed);
        self.decrement_pending(count);
        *self.last_publish_time.write().await = Some(Instant::now());
    }

    /// Record a failed publish
    pub(crate) fn record_failure(&self) {
        self.failed_publishes.fetch_add(1, Ordering::Relaxed);
    }

    /// Record health check result
    pub(crate) async fn record_health_check(&self, ok: bool) {
        self.last_health_check_ok.store(ok, Ordering::Relaxed);
        if ok {
            *self.last_health_check_time.write().await = Some(Instant::now());
        }
    }
}

/// Handle for accessing replication stats
pub type StatsHandle = Arc<ReplicationStats>;

/// Create a new stats handle
pub fn new_stats_handle() -> StatsHandle {
    Arc::new(ReplicationStats::new())
}

/// Snapshot of replication statistics at a point in time
#[derive(Debug, Clone)]
pub struct StatsSnapshot {
    /// Number of changesets pending in buffer
    pub pending_changesets: usize,
    /// Total changesets successfully published
    pub published_changesets: u64,
    /// Total failed publish attempts
    pub failed_publishes: u64,
    /// Whether sequencer is reachable
    pub is_healthy: bool,
}

impl StatsSnapshot {
    /// Create snapshot from stats handle
    pub fn from_stats(stats: &ReplicationStats) -> Self {
        Self {
            pending_changesets: stats.pending_count(),
            published_changesets: stats.published_count(),
            failed_publishes: stats.failed_count(),
            is_healthy: stats.is_healthy(),
        }
    }
}
