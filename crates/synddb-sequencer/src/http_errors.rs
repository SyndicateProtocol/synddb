//! HTTP error handling utilities
//!
//! This module provides:
//! - Type alias for HTTP error responses
//! - Structured error types with automatic conversion

use axum::{http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Standard error response body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Type alias for HTTP error responses
pub type HttpError = (StatusCode, Json<ErrorResponse>);

/// Domain-specific sequencer errors
///
/// All errors automatically convert to appropriate HTTP responses via the
/// `From<SequencerError> for HttpError` implementation, allowing use of the
/// `?` operator for clean error propagation.
#[derive(Debug, Error)]
pub enum SequencerError {
    // ========================================================================
    // External library errors (with automatic `From` conversion)
    // ========================================================================
    #[error("Attestation verification failed: {0}")]
    Attestation(#[from] crate::attestation::AttestationError),

    #[error("Signing failed: {0}")]
    Signing(#[from] crate::signer::SignerError),

    #[error("Serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),

    // ========================================================================
    // Validation errors (manual construction)
    // ========================================================================
    #[error("Attestation token required but not provided")]
    MissingAttestationToken,

    #[error("Invalid recipient address format: must be 0x followed by 40 hex characters")]
    InvalidRecipientAddress,

    #[error("Invalid amount format: must be a non-negative decimal number without leading zeros")]
    InvalidAmount,

    #[error("request_id cannot be empty")]
    EmptyRequestId,

    // ========================================================================
    // Message retrieval errors
    // ========================================================================
    #[error("Message with sequence {0} not found")]
    MessageNotFound(u64),

    #[error("No publisher configured - message retrieval unavailable")]
    NoPublisher,

    #[error("Failed to retrieve message: {0}")]
    MessageRetrievalFailed(String),
}

/// Automatic conversion from `SequencerError` to HTTP response
///
/// This allows the `?` operator to automatically convert domain errors:
/// ```ignore
/// verify_attestation(&token)?;  // AttestationError -> SequencerError -> HttpError
/// serde_json::to_vec(&data)?;   // serde_json::Error -> SequencerError -> HttpError
/// signer.sign_message(&msg)?;   // alloy::signers::Error -> SequencerError -> HttpError
/// return Err(SequencerError::InvalidAmount.into());  // Manual construction
/// ```
impl From<SequencerError> for HttpError {
    fn from(err: SequencerError) -> Self {
        let (status, message) = match err {
            // 401 Unauthorized
            SequencerError::Attestation(_) | SequencerError::MissingAttestationToken => {
                (StatusCode::UNAUTHORIZED, err.to_string())
            }

            // 400 Bad Request
            SequencerError::InvalidRecipientAddress
            | SequencerError::InvalidAmount
            | SequencerError::EmptyRequestId => (StatusCode::BAD_REQUEST, err.to_string()),

            // 404 Not Found
            SequencerError::MessageNotFound(_) => (StatusCode::NOT_FOUND, err.to_string()),

            // 501 Not Implemented
            SequencerError::NoPublisher => (StatusCode::NOT_IMPLEMENTED, err.to_string()),

            // 500 Internal Server Error
            SequencerError::Signing(_)
            | SequencerError::Serialization(_)
            | SequencerError::MessageRetrievalFailed(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
            }
        };

        (status, Json(ErrorResponse { error: message }))
    }
}
