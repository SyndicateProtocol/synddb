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
    /// Number of changesets successfully pushed
    pub(crate) pushed_changesets: AtomicU64,
    /// Number of failed push attempts
    pub(crate) failed_pushes: AtomicU64,
    /// Whether the last health check succeeded
    pub(crate) last_health_check_ok: AtomicBool,
    /// Time of last successful push (protected by `RwLock` for `Instant`)
    pub(crate) last_push_time: RwLock<Option<Instant>>,
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
            pushed_changesets: AtomicU64::new(0),
            failed_pushes: AtomicU64::new(0),
            last_health_check_ok: AtomicBool::new(false),
            last_push_time: RwLock::new(None),
            last_health_check_time: RwLock::new(None),
        }
    }

    /// Get number of changesets pending in buffer
    pub fn pending_count(&self) -> usize {
        self.pending_changesets.load(Ordering::Relaxed)
    }

    /// Get total number of successfully pushed changesets
    pub fn pushed_count(&self) -> u64 {
        self.pushed_changesets.load(Ordering::Relaxed)
    }

    /// Get number of failed push attempts
    pub fn failed_count(&self) -> u64 {
        self.failed_pushes.load(Ordering::Relaxed)
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

    /// Record a successful push
    pub(crate) async fn record_push(&self, count: usize) {
        self.pushed_changesets
            .fetch_add(count as u64, Ordering::Relaxed);
        self.decrement_pending(count);
        *self.last_push_time.write().await = Some(Instant::now());
    }

    /// Record a failed push
    pub(crate) fn record_failure(&self) {
        self.failed_pushes.fetch_add(1, Ordering::Relaxed);
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
    /// Total changesets successfully pushed
    pub pushed_changesets: u64,
    /// Total failed push attempts
    pub failed_pushes: u64,
    /// Whether sequencer is reachable
    pub is_healthy: bool,
}

impl StatsSnapshot {
    /// Create snapshot from stats handle
    pub fn from_stats(stats: &ReplicationStats) -> Self {
        Self {
            pending_changesets: stats.pending_count(),
            pushed_changesets: stats.pushed_count(),
            failed_pushes: stats.failed_count(),
            is_healthy: stats.is_healthy(),
        }
    }
}
