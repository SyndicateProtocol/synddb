use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValidationError {
    // Replay & Nonce
    #[error("REPLAY_DETECTED: message {0} already processed")]
    ReplayDetected(String),
    #[error("INVALID_NONCE: expected {expected}, got {provided} for domain {domain}")]
    InvalidNonce {
        domain: String,
        expected: u64,
        provided: u64,
    },

    // Timestamp
    #[error("TIMESTAMP_EXPIRED: message timestamp {timestamp} outside drift window")]
    TimestampExpired { timestamp: u64 },
    #[error("TIMESTAMP_FUTURE: message timestamp {timestamp} is in the future")]
    TimestampFuture { timestamp: u64 },

    // Authorization
    #[error("APP_NOT_AUTHORIZED: domain {0} not registered")]
    AppNotAuthorized(String),
    #[error("APP_INACTIVE: domain {0} is not active")]
    AppInactive(String),
    #[error("MESSAGE_TYPE_NOT_REGISTERED: {0}")]
    MessageTypeNotRegistered(String),
    #[error("MESSAGE_TYPE_INACTIVE: {0}")]
    MessageTypeInactive(String),

    // Validation
    #[error("CALLDATA_INVALID: {0}")]
    CalldataInvalid(String),
    #[error("SCHEMA_VALIDATION_FAILED: {0}")]
    SchemaValidationFailed(String),
    #[error("METADATA_HASH_MISMATCH: expected {expected}, got {computed}")]
    MetadataHashMismatch { expected: String, computed: String },

    // Invariants
    #[error("INVARIANT_VIOLATED: {invariant} - {message}")]
    InvariantViolated { invariant: String, message: String },
    #[error("INVARIANT_DATA_STALE: {data_source} data older than {max_age_seconds}s")]
    InvariantDataStale {
        data_source: String,
        max_age_seconds: u64,
    },
    #[error("INVARIANT_DATA_UNAVAILABLE: could not fetch {data_source}")]
    InvariantDataUnavailable { data_source: String },

    // Rate limiting
    #[error("RATE_LIMIT_EXCEEDED: {0}")]
    RateLimitExceeded(String),

    // Storage/Bridge
    #[error("STORAGE_PUBLISH_FAILED: {0}")]
    StoragePublishFailed(String),
    #[error("BRIDGE_SUBMIT_FAILED: {0}")]
    BridgeSubmitFailed(String),
    #[error("BRIDGE_CONNECTION_FAILED: {0}")]
    BridgeConnectionFailed(String),

    // Signing
    #[error("SIGNING_FAILED: {0}")]
    SigningFailed(String),

    // Internal
    #[error("INTERNAL_ERROR: {0}")]
    Internal(String),
}

impl ValidationError {
    pub const fn error_code(&self) -> &'static str {
        match self {
            Self::ReplayDetected(_) => "REPLAY_DETECTED",
            Self::InvalidNonce { .. } => "INVALID_NONCE",
            Self::TimestampExpired { .. } => "TIMESTAMP_EXPIRED",
            Self::TimestampFuture { .. } => "TIMESTAMP_FUTURE",
            Self::AppNotAuthorized(_) => "APP_NOT_AUTHORIZED",
            Self::AppInactive(_) => "APP_INACTIVE",
            Self::MessageTypeNotRegistered(_) => "MESSAGE_TYPE_NOT_REGISTERED",
            Self::MessageTypeInactive(_) => "MESSAGE_TYPE_INACTIVE",
            Self::CalldataInvalid(_) => "CALLDATA_INVALID",
            Self::SchemaValidationFailed(_) => "SCHEMA_VALIDATION_FAILED",
            Self::MetadataHashMismatch { .. } => "METADATA_HASH_MISMATCH",
            Self::InvariantViolated { .. } => "INVARIANT_VIOLATED",
            Self::InvariantDataStale { .. } => "INVARIANT_DATA_STALE",
            Self::InvariantDataUnavailable { .. } => "INVARIANT_DATA_UNAVAILABLE",
            Self::RateLimitExceeded(_) => "RATE_LIMIT_EXCEEDED",
            Self::StoragePublishFailed(_) => "STORAGE_PUBLISH_FAILED",
            Self::BridgeSubmitFailed(_) => "BRIDGE_SUBMIT_FAILED",
            Self::BridgeConnectionFailed(_) => "BRIDGE_CONNECTION_FAILED",
            Self::SigningFailed(_) => "SIGNING_FAILED",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }

    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::StoragePublishFailed(_)
                | Self::BridgeSubmitFailed(_)
                | Self::BridgeConnectionFailed(_)
                | Self::InvariantDataUnavailable { .. }
                | Self::Internal(_)
        )
    }
}
