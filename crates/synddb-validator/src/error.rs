//! Error types for the validator

use thiserror::Error;

/// Errors that can occur during validation
#[derive(Debug, Error)]
pub enum ValidatorError {
    // Signature verification errors
    #[error("Payload hash mismatch: expected {expected}, computed {computed}")]
    PayloadHashMismatch { expected: String, computed: String },

    #[error("Signer mismatch: expected {expected}, got {actual}")]
    SignerMismatch { expected: String, actual: String },

    #[error("Signature verification failed: {0}")]
    SignatureVerification(String),

    #[error("Invalid signature format: {0}")]
    InvalidSignature(String),

    // Sync errors
    #[error("Fetch error for sequence {sequence}: {reason}")]
    FetchError { sequence: u64, reason: String },

    #[error("Message not found: sequence {0}")]
    MessageNotFound(u64),

    #[error("Sequence gap detected: expected {expected}, got {actual}")]
    SequenceGap { expected: u64, actual: u64 },

    // Application errors
    #[error("Decompression error: {0}")]
    DecompressionError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Changeset application failed at sequence {sequence}: {reason}")]
    ChangesetApplyError { sequence: u64, reason: String },

    #[error("Changeset conflict: {0}")]
    ChangesetConflict(String),

    // Database errors
    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("State persistence error: {0}")]
    StatePersistenceError(String),

    // Configuration errors
    #[error("Configuration error: {0}")]
    ConfigError(String),
}
