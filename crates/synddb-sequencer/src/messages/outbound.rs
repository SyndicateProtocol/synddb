//! Outbound message monitor
//!
//! Monitors the application's `message_log` table for pending outbound messages
//! and tracks their status. In production, this would also handle submission
//! to the blockchain via Bridge.sol.
//!
//! # Architecture
//!
//! ```text
//! App `SQLite` (`message_log`)
//!     │
//!     │ SELECT WHERE status = 'pending'
//!     ▼
//! OutboundMonitor (polling, read-only)
//!     │
//!     ▼
//! OutboundTracker (in-memory state)
//!     │
//!     ▼ GET /messages/outbound/:id/status
//! Client API
//! ```

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;
use tracing::{debug, info, warn};

/// Configuration for the outbound monitor
#[derive(Debug, Clone)]
pub struct OutboundMonitorConfig {
    /// Path to the application's `SQLite` database
    pub db_path: String,
    /// How often to poll for new messages (default: 1 second)
    pub poll_interval: Duration,
    /// Maximum messages to process per poll (default: 100)
    pub batch_size: usize,
}

impl Default for OutboundMonitorConfig {
    fn default() -> Self {
        Self {
            db_path: String::new(),
            poll_interval: Duration::from_secs(1),
            batch_size: 100,
        }
    }
}

/// Status of an outbound message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutboundStatus {
    /// Message is pending in the app's database
    Pending,
    /// Message has been picked up by the monitor
    Queued,
    /// Message is being submitted to the blockchain
    Submitting,
    /// Transaction has been submitted, waiting for confirmation
    Submitted,
    /// Transaction is confirmed
    Confirmed,
    /// Submission failed
    Failed,
}

impl std::fmt::Display for OutboundStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Queued => write!(f, "queued"),
            Self::Submitting => write!(f, "submitting"),
            Self::Submitted => write!(f, "submitted"),
            Self::Confirmed => write!(f, "confirmed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Tracked state of an outbound message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedOutboundMessage {
    /// Message ID from app's `message_log`
    pub id: u64,
    /// Type of message (`price_update`, `batch_price_update`, `price_response`)
    pub message_type: String,
    /// JSON payload
    pub payload: String,
    /// Current status
    pub status: OutboundStatus,
    /// Transaction hash (if submitted)
    pub tx_hash: Option<String>,
    /// Number of confirmations (if submitted)
    pub confirmations: Option<u64>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// When the message was first seen
    pub first_seen_at: u64,
    /// When the status was last updated
    pub updated_at: u64,
}

/// Thread-safe tracker for outbound messages
#[derive(Debug, Clone)]
pub struct OutboundTracker {
    messages: Arc<RwLock<HashMap<u64, TrackedOutboundMessage>>>,
}

impl Default for OutboundTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl OutboundTracker {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Track a new message from the app's database
    pub fn track_message(&self, id: u64, message_type: String, payload: String) {
        let now = current_timestamp();
        let mut messages = self.messages.write().unwrap();

        messages
            .entry(id)
            .or_insert_with(|| TrackedOutboundMessage {
                id,
                message_type,
                payload,
                status: OutboundStatus::Queued,
                tx_hash: None,
                confirmations: None,
                error: None,
                first_seen_at: now,
                updated_at: now,
            });
    }

    /// Update the status of a message
    pub fn update_status(&self, id: u64, status: OutboundStatus) {
        let now = current_timestamp();
        let mut messages = self.messages.write().unwrap();

        if let Some(msg) = messages.get_mut(&id) {
            msg.status = status;
            msg.updated_at = now;
        }
    }

    /// Set transaction hash for a submitted message
    pub fn set_tx_hash(&self, id: u64, tx_hash: String) {
        let now = current_timestamp();
        let mut messages = self.messages.write().unwrap();

        if let Some(msg) = messages.get_mut(&id) {
            msg.tx_hash = Some(tx_hash);
            msg.status = OutboundStatus::Submitted;
            msg.updated_at = now;
        }
    }

    /// Set confirmations for a submitted message
    pub fn set_confirmations(&self, id: u64, confirmations: u64) {
        let now = current_timestamp();
        let mut messages = self.messages.write().unwrap();

        if let Some(msg) = messages.get_mut(&id) {
            msg.confirmations = Some(confirmations);
            msg.updated_at = now;
        }
    }

    /// Mark a message as confirmed
    pub fn mark_confirmed(&self, id: u64) {
        self.update_status(id, OutboundStatus::Confirmed);
    }

    /// Mark a message as failed with an error
    pub fn mark_failed(&self, id: u64, error: String) {
        let now = current_timestamp();
        let mut messages = self.messages.write().unwrap();

        if let Some(msg) = messages.get_mut(&id) {
            msg.status = OutboundStatus::Failed;
            msg.error = Some(error);
            msg.updated_at = now;
        }
    }

    /// Get the status of a message
    pub fn get_status(&self, id: u64) -> Option<TrackedOutboundMessage> {
        let messages = self.messages.read().unwrap();
        messages.get(&id).cloned()
    }

    /// Get all tracked messages
    pub fn get_all(&self) -> Vec<TrackedOutboundMessage> {
        let messages = self.messages.read().unwrap();
        messages.values().cloned().collect()
    }

    /// Get statistics
    pub fn stats(&self) -> OutboundStats {
        let messages = self.messages.read().unwrap();

        let mut stats = OutboundStats::default();
        for msg in messages.values() {
            stats.total += 1;
            match msg.status {
                OutboundStatus::Pending => stats.pending += 1,
                OutboundStatus::Queued => stats.queued += 1,
                OutboundStatus::Submitting => stats.submitting += 1,
                OutboundStatus::Submitted => stats.submitted += 1,
                OutboundStatus::Confirmed => stats.confirmed += 1,
                OutboundStatus::Failed => stats.failed += 1,
            }
        }
        stats
    }

    /// Remove old confirmed/failed messages to prevent memory growth
    pub fn cleanup_old_messages(&self, max_age_secs: u64) {
        let now = current_timestamp();
        let cutoff = now.saturating_sub(max_age_secs);

        let mut messages = self.messages.write().unwrap();
        messages.retain(|_, msg| {
            // Keep if not terminal state or if recently updated
            !matches!(
                msg.status,
                OutboundStatus::Confirmed | OutboundStatus::Failed
            ) || msg.updated_at > cutoff
        });
    }
}

/// Statistics for outbound messages
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutboundStats {
    pub total: usize,
    pub pending: usize,
    pub queued: usize,
    pub submitting: usize,
    pub submitted: usize,
    pub confirmed: usize,
    pub failed: usize,
}

/// Outbound message monitor that polls the app's `SQLite` database
pub struct OutboundMonitor {
    config: OutboundMonitorConfig,
    tracker: OutboundTracker,
    last_seen_id: u64,
}

impl std::fmt::Debug for OutboundMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboundMonitor")
            .field("config", &self.config)
            .field("last_seen_id", &self.last_seen_id)
            .finish()
    }
}

impl OutboundMonitor {
    pub fn new(config: OutboundMonitorConfig) -> Self {
        Self {
            config,
            tracker: OutboundTracker::new(),
            last_seen_id: 0,
        }
    }

    /// Get a clone of the tracker for sharing with API handlers
    pub fn tracker(&self) -> OutboundTracker {
        self.tracker.clone()
    }

    /// Poll the database once for new pending messages
    pub fn poll_once(&mut self) -> Result<usize, String> {
        if self.config.db_path.is_empty() {
            return Ok(0);
        }

        // Open database in read-only mode
        let conn = Connection::open_with_flags(
            &self.config.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("Failed to open database: {e}"))?;

        // Check if message_log table exists
        let table_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='message_log')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !table_exists {
            debug!("message_log table not found, skipping poll");
            return Ok(0);
        }

        // Query for pending messages after our last seen ID
        let mut stmt = conn
            .prepare(
                "SELECT id, message_type, payload FROM message_log
                 WHERE status = 'pending' AND id > ?
                 ORDER BY id ASC
                 LIMIT ?",
            )
            .map_err(|e| format!("Failed to prepare statement: {e}"))?;

        let messages: Vec<(u64, String, String)> = stmt
            .query_map(
                [self.last_seen_id as i64, self.config.batch_size as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| format!("Failed to query messages: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        let count = messages.len();

        for (id, msg_type, payload) in messages {
            self.tracker.track_message(id, msg_type.clone(), payload);
            self.last_seen_id = self.last_seen_id.max(id);
            debug!(id = id, message_type = %msg_type, "Tracked outbound message");
        }

        if count > 0 {
            info!(count = count, "Polled new outbound messages");
        }

        Ok(count)
    }

    /// Run the monitor loop until shutdown signal
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        info!(
            db_path = %self.config.db_path,
            poll_interval_ms = self.config.poll_interval.as_millis(),
            "Starting outbound message monitor"
        );

        let mut interval = tokio::time::interval(self.config.poll_interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.poll_once() {
                        warn!(error = %e, "Failed to poll outbound messages");
                    }

                    // Clean up old terminal messages every poll
                    self.tracker.cleanup_old_messages(3600); // 1 hour
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Outbound monitor shutting down");
                        break;
                    }
                }
            }
        }
    }
}

/// Handle for the outbound monitor that can be used from API handlers
#[derive(Clone, Debug)]
pub struct OutboundMonitorHandle {
    tracker: OutboundTracker,
}

impl OutboundMonitorHandle {
    pub const fn new(tracker: OutboundTracker) -> Self {
        Self { tracker }
    }

    /// Get the status of a message
    pub fn get_status(&self, id: u64) -> Option<TrackedOutboundMessage> {
        self.tracker.get_status(id)
    }

    /// Get all tracked messages
    pub fn get_all(&self) -> Vec<TrackedOutboundMessage> {
        self.tracker.get_all()
    }

    /// Get statistics
    pub fn stats(&self) -> OutboundStats {
        self.tracker.stats()
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_test_db() -> (tempfile::TempDir, String) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db").to_string_lossy().to_string();

        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE message_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                idempotency_key TEXT UNIQUE,
                status TEXT DEFAULT 'pending',
                created_at INTEGER DEFAULT (unixepoch()),
                trigger_event TEXT,
                trigger_id TEXT,
                submitted_at INTEGER,
                tx_hash TEXT,
                error TEXT
            );
            ",
        )
        .unwrap();

        (dir, db_path)
    }

    #[test]
    fn test_tracker_basic() {
        let tracker = OutboundTracker::new();

        tracker.track_message(1, "price_update".to_string(), "{}".to_string());

        let status = tracker.get_status(1).unwrap();
        assert_eq!(status.id, 1);
        assert_eq!(status.status, OutboundStatus::Queued);
        assert!(status.tx_hash.is_none());
    }

    #[test]
    fn test_tracker_status_updates() {
        let tracker = OutboundTracker::new();

        tracker.track_message(1, "price_update".to_string(), "{}".to_string());

        tracker.set_tx_hash(1, "0xabc123".to_string());
        let status = tracker.get_status(1).unwrap();
        assert_eq!(status.status, OutboundStatus::Submitted);
        assert_eq!(status.tx_hash, Some("0xabc123".to_string()));

        tracker.set_confirmations(1, 6);
        let status = tracker.get_status(1).unwrap();
        assert_eq!(status.confirmations, Some(6));

        tracker.mark_confirmed(1);
        let status = tracker.get_status(1).unwrap();
        assert_eq!(status.status, OutboundStatus::Confirmed);
    }

    #[test]
    fn test_tracker_mark_failed() {
        let tracker = OutboundTracker::new();

        tracker.track_message(1, "price_update".to_string(), "{}".to_string());
        tracker.mark_failed(1, "Transaction reverted".to_string());

        let status = tracker.get_status(1).unwrap();
        assert_eq!(status.status, OutboundStatus::Failed);
        assert_eq!(status.error, Some("Transaction reverted".to_string()));
    }

    #[test]
    fn test_tracker_stats() {
        let tracker = OutboundTracker::new();

        tracker.track_message(1, "a".to_string(), "{}".to_string());
        tracker.track_message(2, "b".to_string(), "{}".to_string());
        tracker.track_message(3, "c".to_string(), "{}".to_string());

        tracker.mark_confirmed(1);
        tracker.mark_failed(2, "error".to_string());

        let stats = tracker.stats();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.queued, 1);
        assert_eq!(stats.confirmed, 1);
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn test_monitor_poll() {
        let (_dir, db_path) = setup_test_db();

        // Insert some test messages
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO message_log (message_type, payload, status) VALUES (?, ?, ?)",
            ["price_update", r#"{"asset":"BTC"}"#, "pending"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_log (message_type, payload, status) VALUES (?, ?, ?)",
            ["price_update", r#"{"asset":"ETH"}"#, "pending"],
        )
        .unwrap();
        // This one should not be picked up (already submitted)
        conn.execute(
            "INSERT INTO message_log (message_type, payload, status) VALUES (?, ?, ?)",
            ["price_update", r#"{"asset":"SOL"}"#, "submitted"],
        )
        .unwrap();
        drop(conn);

        let config = OutboundMonitorConfig {
            db_path,
            poll_interval: Duration::from_millis(100),
            batch_size: 100,
        };

        let mut monitor = OutboundMonitor::new(config);
        let count = monitor.poll_once().unwrap();

        assert_eq!(count, 2);

        let all = monitor.tracker().get_all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_monitor_incremental_poll() {
        let (_dir, db_path) = setup_test_db();

        let config = OutboundMonitorConfig {
            db_path: db_path.clone(),
            poll_interval: Duration::from_millis(100),
            batch_size: 100,
        };

        let mut monitor = OutboundMonitor::new(config);

        // First poll - empty
        let count = monitor.poll_once().unwrap();
        assert_eq!(count, 0);

        // Insert a message
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO message_log (message_type, payload, status) VALUES (?, ?, ?)",
            ["price_update", r#"{"asset":"BTC"}"#, "pending"],
        )
        .unwrap();
        drop(conn);

        // Second poll - should find 1
        let count = monitor.poll_once().unwrap();
        assert_eq!(count, 1);

        // Third poll - should find 0 (same message)
        let count = monitor.poll_once().unwrap();
        assert_eq!(count, 0);

        // Insert another message
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO message_log (message_type, payload, status) VALUES (?, ?, ?)",
            ["price_update", r#"{"asset":"ETH"}"#, "pending"],
        )
        .unwrap();
        drop(conn);

        // Fourth poll - should find 1 new message
        let count = monitor.poll_once().unwrap();
        assert_eq!(count, 1);

        assert_eq!(monitor.tracker().get_all().len(), 2);
    }
}
