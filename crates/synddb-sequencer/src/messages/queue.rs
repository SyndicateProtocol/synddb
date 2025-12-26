//! Message queue for inbound and outbound messages
//!
//! Provides in-memory storage for messages with:
//! - Bounded capacity to prevent memory exhaustion
//! - Retention policy (configurable, default 24 hours)
//! - Thread-safe access via `Arc<RwLock<_>>`

use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Configuration for the message queue
#[derive(Debug, Clone)]
pub struct MessageQueueConfig {
    /// Maximum number of messages to retain (default: 10,000)
    pub max_size: usize,
    /// Retention period in seconds (default: 24 hours)
    pub retention_secs: u64,
}

impl Default for MessageQueueConfig {
    fn default() -> Self {
        Self {
            max_size: 10_000,
            retention_secs: 24 * 60 * 60, // 24 hours
        }
    }
}

/// An inbound message from the blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// Internal sequence ID (assigned by sequencer)
    pub id: u64,
    /// Message ID from the blockchain event (e.g., requestId)
    pub message_id: String,
    /// Type of message (e.g., "price_request", "deposit")
    pub message_type: String,
    /// JSON payload with message-specific data
    pub payload: String,
    /// Sender address on blockchain
    pub sender: String,
    /// Transaction hash where event was emitted
    pub tx_hash: String,
    /// Block number where event was emitted
    pub block_number: u64,
    /// Number of confirmations
    pub confirmations: u64,
    /// Timestamp when message was captured
    pub timestamp: u64,
    /// Whether this message has been acknowledged by the app
    pub acknowledged: bool,
    /// When acknowledged (if applicable)
    pub acknowledged_at: Option<u64>,
}

/// Status of an outbound message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessageStatus {
    /// Message ID from the app's message_log table
    pub id: u64,
    /// Current status: "pending", "submitted", "confirmed", "failed"
    pub status: String,
    /// Transaction hash (if submitted)
    pub tx_hash: Option<String>,
    /// Number of confirmations (if submitted)
    pub confirmations: Option<u64>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// Timestamp of last status update
    pub updated_at: u64,
}

/// Thread-safe message queue
pub struct MessageQueue {
    config: MessageQueueConfig,
    messages: VecDeque<InboundMessage>,
    next_id: AtomicU64,
}

impl std::fmt::Debug for MessageQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageQueue")
            .field("config", &self.config)
            .field("message_count", &self.messages.len())
            .field("next_id", &self.next_id.load(Ordering::SeqCst))
            .finish()
    }
}

impl MessageQueue {
    /// Create a new message queue with default configuration
    pub fn new() -> Self {
        Self::with_config(MessageQueueConfig::default())
    }

    /// Create a new message queue with custom configuration
    pub fn with_config(config: MessageQueueConfig) -> Self {
        Self {
            config,
            messages: VecDeque::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Add a new inbound message to the queue
    ///
    /// Returns the assigned message ID
    pub fn add_message(
        &mut self,
        message_id: String,
        message_type: String,
        payload: String,
        sender: String,
        tx_hash: String,
        block_number: u64,
        confirmations: u64,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let message = InboundMessage {
            id,
            message_id,
            message_type,
            payload,
            sender,
            tx_hash,
            block_number,
            confirmations,
            timestamp,
            acknowledged: false,
            acknowledged_at: None,
        };

        self.messages.push_back(message);

        // Enforce max size by removing oldest messages
        while self.messages.len() > self.config.max_size {
            self.messages.pop_front();
        }

        // Clean up expired messages
        self.cleanup_expired();

        id
    }

    /// Get messages after a given ID
    ///
    /// Returns up to `limit` messages with ID > `after_id`
    pub fn get_messages_after(&self, after_id: u64, limit: usize) -> Vec<&InboundMessage> {
        self.messages
            .iter()
            .filter(|m| m.id > after_id)
            .take(limit)
            .collect()
    }

    /// Get all pending (unacknowledged) messages
    pub fn get_pending_messages(&self, limit: usize) -> Vec<&InboundMessage> {
        self.messages
            .iter()
            .filter(|m| !m.acknowledged)
            .take(limit)
            .collect()
    }

    /// Get pending messages of a specific type
    pub fn get_pending_by_type(&self, message_type: &str, limit: usize) -> Vec<&InboundMessage> {
        self.messages
            .iter()
            .filter(|m| !m.acknowledged && m.message_type == message_type)
            .take(limit)
            .collect()
    }

    /// Acknowledge a message by ID
    ///
    /// Returns true if the message was found and acknowledged
    pub fn acknowledge(&mut self, id: u64) -> bool {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) {
            if !msg.acknowledged {
                msg.acknowledged = true;
                msg.acknowledged_at = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                );
                return true;
            }
        }
        false
    }

    /// Get a message by ID
    pub fn get_by_id(&self, id: u64) -> Option<&InboundMessage> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// Get the latest message ID
    pub fn latest_id(&self) -> u64 {
        self.messages.back().map_or(0, |m| m.id)
    }

    /// Get queue statistics
    pub fn stats(&self) -> QueueStats {
        let total = self.messages.len();
        let pending = self.messages.iter().filter(|m| !m.acknowledged).count();
        let acknowledged = total - pending;

        QueueStats {
            total,
            pending,
            acknowledged,
            max_size: self.config.max_size,
        }
    }

    /// Remove expired messages based on retention policy
    fn cleanup_expired(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let cutoff = now.saturating_sub(self.config.retention_secs);

        // Remove messages older than retention period
        // Only remove if they're acknowledged (don't lose pending messages)
        while let Some(front) = self.messages.front() {
            if front.acknowledged && front.timestamp < cutoff {
                self.messages.pop_front();
            } else {
                break;
            }
        }
    }
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Queue statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStats {
    /// Total messages in queue
    pub total: usize,
    /// Pending (unacknowledged) messages
    pub pending: usize,
    /// Acknowledged messages
    pub acknowledged: usize,
    /// Maximum queue size
    pub max_size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_get_messages() {
        let mut queue = MessageQueue::new();

        let id1 = queue.add_message(
            "req-1".to_string(),
            "price_request".to_string(),
            r#"{"asset":"BTC"}"#.to_string(),
            "0x123".to_string(),
            "0xabc".to_string(),
            100,
            12,
        );

        let id2 = queue.add_message(
            "req-2".to_string(),
            "price_request".to_string(),
            r#"{"asset":"ETH"}"#.to_string(),
            "0x456".to_string(),
            "0xdef".to_string(),
            101,
            12,
        );

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);

        let messages = queue.get_messages_after(0, 10);
        assert_eq!(messages.len(), 2);

        let messages = queue.get_messages_after(1, 10);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 2);
    }

    #[test]
    fn test_acknowledge_message() {
        let mut queue = MessageQueue::new();

        let id = queue.add_message(
            "req-1".to_string(),
            "price_request".to_string(),
            r#"{}"#.to_string(),
            "0x123".to_string(),
            "0xabc".to_string(),
            100,
            12,
        );

        // Initially pending
        let pending = queue.get_pending_messages(10);
        assert_eq!(pending.len(), 1);

        // Acknowledge
        assert!(queue.acknowledge(id));

        // No longer pending
        let pending = queue.get_pending_messages(10);
        assert_eq!(pending.len(), 0);

        // Can still get by ID
        let msg = queue.get_by_id(id).unwrap();
        assert!(msg.acknowledged);
        assert!(msg.acknowledged_at.is_some());
    }

    #[test]
    fn test_max_size_enforcement() {
        let config = MessageQueueConfig {
            max_size: 3,
            retention_secs: 86400,
        };
        let mut queue = MessageQueue::with_config(config);

        for i in 0..5 {
            queue.add_message(
                format!("req-{i}"),
                "test".to_string(),
                "{}".to_string(),
                "0x".to_string(),
                "0x".to_string(),
                i as u64,
                0,
            );
        }

        // Should only have last 3 messages
        let stats = queue.stats();
        assert_eq!(stats.total, 3);

        // First two should be gone
        assert!(queue.get_by_id(1).is_none());
        assert!(queue.get_by_id(2).is_none());
        // Last three should exist
        assert!(queue.get_by_id(3).is_some());
        assert!(queue.get_by_id(4).is_some());
        assert!(queue.get_by_id(5).is_some());
    }

    #[test]
    fn test_get_pending_by_type() {
        let mut queue = MessageQueue::new();

        queue.add_message(
            "req-1".to_string(),
            "price_request".to_string(),
            "{}".to_string(),
            "0x".to_string(),
            "0x".to_string(),
            100,
            0,
        );

        queue.add_message(
            "dep-1".to_string(),
            "deposit".to_string(),
            "{}".to_string(),
            "0x".to_string(),
            "0x".to_string(),
            101,
            0,
        );

        queue.add_message(
            "req-2".to_string(),
            "price_request".to_string(),
            "{}".to_string(),
            "0x".to_string(),
            "0x".to_string(),
            102,
            0,
        );

        let price_requests = queue.get_pending_by_type("price_request", 10);
        assert_eq!(price_requests.len(), 2);

        let deposits = queue.get_pending_by_type("deposit", 10);
        assert_eq!(deposits.len(), 1);
    }

    #[test]
    fn test_stats() {
        let mut queue = MessageQueue::new();

        queue.add_message(
            "req-1".to_string(),
            "test".to_string(),
            "{}".to_string(),
            "0x".to_string(),
            "0x".to_string(),
            100,
            0,
        );

        let id2 = queue.add_message(
            "req-2".to_string(),
            "test".to_string(),
            "{}".to_string(),
            "0x".to_string(),
            "0x".to_string(),
            101,
            0,
        );

        queue.acknowledge(id2);

        let stats = queue.stats();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.acknowledged, 1);
    }
}
