//! Types for Bridge client operations.

use serde::{Deserialize, Serialize};

/// Result of pushing a message to the Bridge validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResult {
    /// Whether the message was accepted.
    pub success: bool,
    /// Message ID if accepted.
    pub message_id: Option<String>,
    /// Validator signature if accepted.
    pub signature: Option<String>,
    /// Error code if rejected.
    pub error_code: Option<String>,
    /// Error message if rejected.
    pub error_message: Option<String>,
    /// Whether the error is retryable.
    pub is_retryable: bool,
    /// Number of attempts made.
    pub attempts: u32,
}

impl PushResult {
    /// Create a successful result.
    pub fn success(message_id: String, signature: Option<String>) -> Self {
        Self {
            success: true,
            message_id: Some(message_id),
            signature,
            error_code: None,
            error_message: None,
            is_retryable: false,
            attempts: 1,
        }
    }

    /// Create a failed result.
    pub fn failure(error_code: &str, error_message: &str, is_retryable: bool) -> Self {
        Self {
            success: false,
            message_id: None,
            signature: None,
            error_code: Some(error_code.to_string()),
            error_message: Some(error_message.to_string()),
            is_retryable,
            attempts: 1,
        }
    }
}

/// Status of a message on the Bridge.
///
/// Message stages:
/// - 0: not_initialized
/// - 1: pending
/// - 2: ready
/// - 3: pre_execution
/// - 4: executing
/// - 5: post_execution
/// - 6: completed
/// - 7: failed
/// - 8: expired
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageStatus {
    /// Message ID.
    pub message_id: String,
    /// Current stage (0-8).
    pub stage: u8,
    /// Human-readable status.
    pub status: String,
    /// Whether execution completed.
    pub executed: bool,
    /// Number of signatures collected.
    pub signatures_collected: u32,
    /// Threshold required.
    pub signature_threshold: u32,
    /// Block number if executed.
    pub block_number: Option<u64>,
    /// Transaction hash if executed.
    pub tx_hash: Option<String>,
}

impl MessageStatus {
    /// Check if the message has reached a terminal state.
    pub const fn is_terminal(&self) -> bool {
        matches!(self.stage, 6 | 7 | 8) // completed, failed, expired
    }

    /// Check if the message was successfully executed.
    pub const fn is_success(&self) -> bool {
        self.stage == 6 // completed
    }

    /// Check if the message failed.
    pub const fn is_failed(&self) -> bool {
        self.stage == 7
    }

    /// Check if the message expired.
    pub const fn is_expired(&self) -> bool {
        self.stage == 8
    }
}

/// Map stage numbers to human-readable status strings.
pub fn stage_to_status(stage: u8) -> &'static str {
    match stage {
        0 => "not_initialized",
        1 => "pending",
        2 => "ready",
        3 => "pre_execution",
        4 => "executing",
        5 => "post_execution",
        6 => "completed",
        7 => "failed",
        8 => "expired",
        _ => "unknown",
    }
}

/// Error codes that indicate the error is transient and can be retried.
pub const RETRYABLE_ERRORS: &[&str] = &[
    "STORAGE_PUBLISH_FAILED",
    "BRIDGE_SUBMIT_FAILED",
    "BRIDGE_CONNECTION_FAILED",
    "INVARIANT_DATA_UNAVAILABLE",
    "INTERNAL_ERROR",
    "CONNECTION_ERROR",
    "TIMEOUT",
];

/// Check if an error code indicates a retryable error.
pub fn is_retryable_error(error_code: &str) -> bool {
    RETRYABLE_ERRORS.contains(&error_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_status_terminal() {
        let pending = MessageStatus {
            message_id: "0x123".to_string(),
            stage: 1,
            status: "pending".to_string(),
            executed: false,
            signatures_collected: 0,
            signature_threshold: 1,
            block_number: None,
            tx_hash: None,
        };
        assert!(!pending.is_terminal());

        let completed = MessageStatus {
            stage: 6,
            ..pending.clone()
        };
        assert!(completed.is_terminal());
        assert!(completed.is_success());

        let failed = MessageStatus {
            stage: 7,
            ..pending
        };
        assert!(failed.is_terminal());
        assert!(failed.is_failed());
    }

    #[test]
    fn test_retryable_errors() {
        assert!(is_retryable_error("CONNECTION_ERROR"));
        assert!(is_retryable_error("TIMEOUT"));
        assert!(!is_retryable_error("INVALID_NONCE"));
        assert!(!is_retryable_error("REPLAY_DETECTED"));
    }
}
