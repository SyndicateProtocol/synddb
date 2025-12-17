//! Error types for CBOR/COSE operations

use thiserror::Error;

/// Errors that can occur during CBOR/COSE operations
#[derive(Debug, Error)]
pub enum CborError {
    /// CBOR serialization/deserialization error
    #[error("CBOR error: {0}")]
    Cbor(String),

    /// COSE structure error
    #[error("COSE error: {0}")]
    Cose(String),

    /// Signature verification failed
    #[error("Signature verification failed: {0}")]
    SignatureVerification(String),

    /// Invalid message type
    #[error("Invalid message type: {0}")]
    InvalidMessageType(u8),

    /// Missing required header field
    #[error("Missing header field: {0}")]
    MissingHeader(String),

    /// Compression/decompression error
    #[error("Compression error: {0}")]
    Compression(String),

    /// Signing operation failed
    #[error("Signing failed: {0}")]
    Signing(String),

    /// Invalid batch structure
    #[error("Invalid batch: {0}")]
    InvalidBatch(String),
}

impl From<ciborium::ser::Error<std::io::Error>> for CborError {
    fn from(e: ciborium::ser::Error<std::io::Error>) -> Self {
        Self::Cbor(e.to_string())
    }
}

impl From<ciborium::de::Error<std::io::Error>> for CborError {
    fn from(e: ciborium::de::Error<std::io::Error>) -> Self {
        Self::Cbor(e.to_string())
    }
}

impl From<coset::CoseError> for CborError {
    fn from(e: coset::CoseError) -> Self {
        Self::Cose(e.to_string())
    }
}

impl From<std::io::Error> for CborError {
    fn from(e: std::io::Error) -> Self {
        Self::Compression(e.to_string())
    }
}
