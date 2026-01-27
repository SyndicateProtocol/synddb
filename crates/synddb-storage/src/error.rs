//! Storage error types

use thiserror::Error;

/// Errors that can occur during storage operations
#[derive(Debug, Error)]
pub enum StorageError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Failed to read from storage
    #[error("Read error: {0}")]
    Read(String),

    /// Failed to write to storage
    #[error("Write error: {0}")]
    Write(String),

    /// Failed to list objects
    #[error("List error: {0}")]
    List(String),

    /// Object not found
    #[error("Object not found: {0}")]
    NotFound(String),

    /// HTTP error (for emulator REST API)
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
}
