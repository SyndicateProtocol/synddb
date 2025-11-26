//! HTTP error handling utilities
//!
//! This module provides:
//! - Type alias for HTTP error responses
//! - Helper functions for common error cases
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

// ============================================================================
// Helper Functions (for simple/ad-hoc errors)
// ============================================================================

/// Create a bad request (400) error
pub fn bad_request(msg: impl Into<String>) -> HttpError {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: msg.into() }),
    )
}

/// Create an unauthorized (401) error
pub fn unauthorized(msg: impl Into<String>) -> HttpError {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse { error: msg.into() }),
    )
}

/// Create an internal server error (500)
pub fn internal_error(msg: impl Into<String>) -> HttpError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: msg.into() }),
    )
}

/// Create a not found (404) error
pub fn not_found(msg: impl Into<String>) -> HttpError {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse { error: msg.into() }),
    )
}

// ============================================================================
// Structured Domain Errors (for recurring/complex cases)
// ============================================================================

/// Domain-specific sequencer errors
///
/// These errors automatically convert to appropriate HTTP responses via the
/// `From<SequencerError> for HttpError` implementation, allowing use of the
/// `?` operator for clean error propagation.
#[derive(Debug, Error)]
pub enum SequencerError {
    #[error("Attestation verification failed: {0}")]
    Attestation(#[from] crate::attestation::AttestationError),

    #[error("Signing failed: {0}")]
    Signing(#[from] alloy::signers::Error),

    #[error("Serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Automatic conversion from `SequencerError` to HTTP response
///
/// This allows the `?` operator to automatically convert domain errors:
/// ```ignore
/// verify_attestation(&token)?;  // AttestationError -> SequencerError -> HttpError
/// serde_json::to_vec(&data)?;   // serde_json::Error -> SequencerError -> HttpError
/// signer.sign_message(&msg)?;   // alloy::signers::Error -> SequencerError -> HttpError
/// ```
impl From<SequencerError> for HttpError {
    fn from(err: SequencerError) -> Self {
        match err {
            SequencerError::Attestation(_) => unauthorized(err.to_string()),
            SequencerError::Signing(_) | SequencerError::Serialization(_) => {
                internal_error(err.to_string())
            }
        }
    }
}
