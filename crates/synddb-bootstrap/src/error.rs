//! Bootstrap error types

use std::time::Duration;

/// Errors that can occur during key bootstrapping
#[derive(Debug, Clone, thiserror::Error)]
pub enum BootstrapError {
    /// Attestation socket not available (not running in Confidential Space)
    #[error("Attestation socket not available: {0}")]
    AttestationSocketMissing(String),

    /// Failed to fetch attestation token
    #[error("Failed to fetch attestation token: {0}")]
    AttestationFetchFailed(String),

    /// Failed to fetch JWKS from Google
    #[error("Failed to fetch JWKS: {0}")]
    JwksFetchFailed(String),

    /// Proof generation failed
    #[error("Proof generation failed: {0}")]
    ProofGenerationFailed(String),

    /// Proof service unavailable
    #[error("Proof service unavailable: {0}")]
    ProofServiceUnavailable(String),

    /// Proof generation timed out
    #[error("Proof generation timed out after {0:?}")]
    ProofGenerationTimeout(Duration),

    /// Contract submission failed
    #[error("Contract submission failed: {0}")]
    ContractSubmissionFailed(String),

    /// Transaction failed
    #[error("Transaction failed: {0}")]
    TransactionFailed(String),

    /// Transaction confirmation failed
    #[error("Transaction confirmation failed: {0}")]
    TransactionConfirmationFailed(String),

    /// Key verification failed after registration
    #[error("Key verification failed: {0}")]
    KeyVerificationFailed(String),

    /// Insufficient balance for gas
    #[error("Insufficient balance: have {have} wei, need {need} wei")]
    InsufficientBalance { have: u128, need: u128 },

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Bootstrap timed out
    #[error("Bootstrap timed out after {0:?}")]
    Timeout(Duration),

    /// Maximum retries exceeded
    #[error("Maximum retries exceeded for {operation}: {last_error}")]
    MaxRetriesExceeded {
        operation: String,
        last_error: String,
    },
}

impl BootstrapError {
    /// Whether this error is likely transient and worth retrying
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::AttestationFetchFailed(_)
                | Self::JwksFetchFailed(_)
                | Self::ProofGenerationFailed(_)
                | Self::ProofServiceUnavailable(_)
                | Self::ContractSubmissionFailed(_)
                | Self::TransactionFailed(_)
                | Self::TransactionConfirmationFailed(_)
        )
    }

    /// Whether this error indicates we should wait for external action
    pub const fn requires_external_action(&self) -> bool {
        matches!(self, Self::InsufficientBalance { .. })
    }
}
