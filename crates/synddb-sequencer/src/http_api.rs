//! HTTP API for the sequencer node
//!
//! This module implements the HTTP API that clients use to submit data
//! to the sequencer for ordering and signing.
//!
//! # Endpoints
//!
//! ## Data Submission
//!
//! | Endpoint | Method | Content-Type | Description |
//! |----------|--------|--------------|-------------|
//! | `/changesets` | POST | `application/cbor` | Submit changeset batch |
//! | `/snapshots` | POST | `application/cbor` | Submit database snapshot |
//! | `/withdrawals` | POST | `application/cbor` | Submit withdrawal request |
//!
//! ## Operational
//!
//! | Endpoint | Method | Description |
//! |----------|--------|-------------|
//! | `/health` | GET | Liveness probe (always returns 200 OK) |
//! | `/ready` | GET | Readiness probe (checks batcher health) |
//! | `/status` | GET | Current sequence number and signer address |
//! | `/batch/stats` | GET | CBOR batch statistics |
//! | `/batch/flush` | POST | Force flush pending batch to storage |
//!
//! # Wire Format
//!
//! All data submission endpoints accept CBOR-encoded payloads with
//! `Content-Type: application/cbor`. See [`synddb_shared::types::payloads`]
//! for payload structures:
//!
//! - [`ChangesetBatchRequest`] for `/changesets`
//! - [`SnapshotRequest`] for `/snapshots`
//! - [`WithdrawalRequest`] for `/withdrawals`
//!
//! # Response Format
//!
//! Successful data submissions return JSON with:
//!
//! ```json
//! {
//!   "sequence": 42,
//!   "timestamp": 1700000000,
//!   "message_hash": "0x...",
//!   "signature": "0x...",
//!   "signer": "0x..."
//! }
//! ```
//!
//! # Error Handling
//!
//! Errors are returned as JSON with appropriate HTTP status codes:
//!
//! - `400 Bad Request` - Invalid payload or parameters
//! - `401 Unauthorized` - Invalid attestation token
//! - `500 Internal Server Error` - Signing or storage failure
//!
//! [`ChangesetBatchRequest`]: synddb_shared::types::payloads::ChangesetBatchRequest
//! [`SnapshotRequest`]: synddb_shared::types::payloads::SnapshotRequest
//! [`WithdrawalRequest`]: synddb_shared::types::payloads::WithdrawalRequest

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::{
    attestation::AttestationVerifier,
    batcher::{BatchStats, BatcherHandle},
    cbor_extractor::Cbor,
    http_errors::{HttpError, SequencerError},
    inbox::{Inbox, SequenceReceipt},
};
use synddb_shared::types::{
    cbor::message::CborMessageType,
    message::{MessageType, SignedMessage},
    payloads::{ChangesetBatchRequest, SnapshotRequest, WithdrawalRequest},
};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub inbox: Arc<Inbox>,
    /// Optional attestation verifier for TEE token validation
    pub attestation_verifier: Option<Arc<AttestationVerifier>>,
    /// Optional batcher for CBOR batch publishing
    pub batcher: Option<BatcherHandle>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("inbox", &self.inbox)
            .field("attestation_verifier", &self.attestation_verifier.is_some())
            .field("batcher", &self.batcher.is_some())
            .finish()
    }
}

/// Create the HTTP router with all endpoints
pub fn create_router(state: AppState) -> Router {
    info!("Endpoints:");
    info!("  POST /changesets       - Submit changeset batch");
    info!("  POST /withdrawals      - Submit withdrawal request");
    info!("  POST /snapshots        - Submit database snapshot");
    info!("  GET  /health           - Health check (liveness)");
    info!("  GET  /ready            - Readiness check");
    info!("  GET  /status           - Sequencer status");
    info!("  GET  /batch/stats      - CBOR batch statistics");
    info!("  POST /batch/flush      - Force flush pending batch");

    Router::new()
        .route("/changesets", post(receive_changesets))
        .route("/withdrawals", post(receive_withdrawal))
        .route("/snapshots", post(receive_snapshot))
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .route("/status", get(status))
        .route("/batch/stats", get(batch_stats))
        .route("/batch/flush", post(batch_flush))
        .with_state(state)
}

// ============================================================================
// Response Types (HTTP API specific)
// ============================================================================

/// Response for successful sequencing
#[derive(Debug, Serialize, Deserialize)]
pub struct SequenceResponse {
    /// Assigned sequence number
    pub sequence: u64,
    /// Timestamp when sequenced
    pub timestamp: u64,
    /// Hash of the message
    pub message_hash: String,
    /// Sequencer signature
    pub signature: String,
    /// Sequencer address
    pub signer: String,
}

impl From<SequenceReceipt> for SequenceResponse {
    fn from(receipt: SequenceReceipt) -> Self {
        Self {
            sequence: receipt.sequence,
            timestamp: receipt.timestamp,
            message_hash: receipt.message_hash,
            signature: receipt.signature,
            signer: receipt.signer,
        }
    }
}

/// Status response
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    /// Current sequence number (next to be assigned)
    pub current_sequence: u64,
    /// Sequencer's Ethereum address (checksummed, with 0x prefix)
    pub signer_address: String,
    /// Sequencer's 64-byte public key (hex encoded with 0x prefix)
    ///
    /// This is the uncompressed secp256k1 public key used for signature verification.
    /// Validators use this to verify COSE signatures on messages.
    pub signer_pubkey: String,
}

/// Individual health check result
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthCheck {
    /// Name of the component being checked
    pub name: String,
    /// Status: `ok`, `degraded`, `not_configured`
    pub status: String,
    /// Optional message with details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Readiness check response
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadinessResponse {
    /// Overall status: "ready" or "degraded"
    pub status: String,
    /// Individual component checks
    pub checks: Vec<HealthCheck>,
}

/// Batch statistics response
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BatchStatsResponse {
    /// Whether batching is enabled
    pub enabled: bool,
    /// Total batches published
    pub batches_published: u64,
    /// Total messages published via batching
    pub messages_published: u64,
    /// Total compressed bytes published
    pub bytes_published: u64,
    /// Total uncompressed bytes (for compression ratio)
    pub bytes_uncompressed: u64,
    /// Average compression ratio
    pub compression_ratio: f64,
    /// Current pending message count
    pub pending_messages: usize,
    /// Current pending byte count
    pub pending_bytes: usize,
    /// Last flush timestamp (epoch seconds, 0 if never flushed)
    pub last_flush_timestamp: u64,
}

impl From<BatchStats> for BatchStatsResponse {
    fn from(stats: BatchStats) -> Self {
        Self {
            enabled: true,
            batches_published: stats.batches_published,
            messages_published: stats.messages_published,
            bytes_published: stats.bytes_published,
            bytes_uncompressed: stats.bytes_uncompressed,
            compression_ratio: stats.compression_ratio(),
            pending_messages: stats.pending_messages,
            pending_bytes: stats.pending_bytes,
            last_flush_timestamp: stats.last_flush_timestamp,
        }
    }
}

/// Batch flush response
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchFlushResponse {
    /// Whether a batch was published
    pub published: bool,
    /// Reference to the published batch (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Number of messages in the flushed batch
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count: Option<usize>,
    /// Compressed size of the batch
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compressed_bytes: Option<usize>,
}

/// Response for message retrieval
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    /// Sequence number
    pub sequence: u64,
    /// Timestamp when sequenced
    pub timestamp: u64,
    /// Message type
    pub message_type: String,
    /// Original payload (raw bytes)
    pub payload: Vec<u8>,
    /// Hash of the message
    pub message_hash: String,
    /// Sequencer signature
    pub signature: String,
    /// Sequencer address
    pub signer: String,
}

impl From<SignedMessage> for MessageResponse {
    fn from(msg: SignedMessage) -> Self {
        Self {
            sequence: msg.sequence,
            timestamp: msg.timestamp,
            message_type: match msg.message_type {
                MessageType::Changeset => "changeset".to_string(),
                MessageType::Withdrawal => "withdrawal".to_string(),
                MessageType::Snapshot => "snapshot".to_string(),
            },
            payload: msg.payload,
            message_hash: msg.message_hash,
            signature: msg.signature,
            signer: msg.signer,
        }
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// Receive and sequence a changeset batch (CBOR format)
async fn receive_changesets(
    State(state): State<AppState>,
    Cbor(request): Cbor<ChangesetBatchRequest>,
) -> Result<impl IntoResponse, HttpError> {
    info!(
        batch_id = %request.batch_id,
        changeset_count = request.changesets.len(),
        "Received changeset batch"
    );

    // Verify attestation token if verifier is configured
    if let Some(verifier) = &state.attestation_verifier {
        match &request.attestation_token {
            Some(token) => {
                verifier
                    .verify(token)
                    .await
                    .map_err(|e| {
                        error!(batch_id = %request.batch_id, error = %e, "Attestation verification failed");
                        SequencerError::from(e)
                    })?;
                info!(batch_id = %request.batch_id, "Attestation token verified");
            }
            None => {
                error!(batch_id = %request.batch_id, "Missing attestation token");
                return Err(SequencerError::MissingAttestationToken.into());
            }
        }
    }

    // Serialize the batch as CBOR payload
    let payload = request.to_cbor().map_err(|e| {
        error!("Failed to serialize changeset batch: {}", e);
        SequencerError::CborSerializationFailed(e.to_string())
    })?;

    // Sequence and sign the message (creates COSE signed message)
    let (cbor_message, receipt) = state
        .inbox
        .sequence_message(CborMessageType::Changeset, payload)
        .map_err(|e| {
            error!("Failed to sequence message: {}", e);
            SequencerError::from(e)
        })?;

    // Send to batcher for batching if configured
    if let Some(batcher) = &state.batcher {
        if let Err(e) = batcher.add_message(cbor_message).await {
            warn!(
                sequence = receipt.sequence,
                error = %e,
                "Failed to add to batcher (sequencing succeeded)"
            );
        }
    }

    info!(
        sequence = receipt.sequence,
        batch_id = %request.batch_id,
        "Changeset batch sequenced"
    );

    Ok((StatusCode::CREATED, Json(SequenceResponse::from(receipt))))
}

/// Receive and sequence a withdrawal request (CBOR format)
async fn receive_withdrawal(
    State(state): State<AppState>,
    Cbor(request): Cbor<WithdrawalRequest>,
) -> Result<impl IntoResponse, HttpError> {
    info!(
        request_id = %request.request_id,
        recipient = %request.recipient,
        amount = %request.amount,
        "Received withdrawal request"
    );

    // Validate recipient address format (0x + 40 hex chars)
    if !request.recipient.starts_with("0x")
        || request.recipient.len() != 42
        || !request.recipient[2..]
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    {
        return Err(SequencerError::InvalidRecipientAddress.into());
    }

    // Validate amount is a valid decimal number (can be parsed as u128 or larger)
    if request.amount.is_empty()
        || !request.amount.chars().all(|c| c.is_ascii_digit())
        || (request.amount.len() > 1 && request.amount.starts_with('0'))
    {
        return Err(SequencerError::InvalidAmount.into());
    }

    // Validate request_id is not empty
    if request.request_id.is_empty() {
        return Err(SequencerError::EmptyRequestId.into());
    }

    // Serialize the request as CBOR payload
    let payload = request.to_cbor().map_err(|e| {
        error!("Failed to serialize withdrawal request: {}", e);
        SequencerError::CborSerializationFailed(e.to_string())
    })?;

    // Sequence and sign the message (creates COSE signed message)
    let (cbor_message, receipt) = state
        .inbox
        .sequence_message(CborMessageType::Withdrawal, payload)
        .map_err(|e| {
            error!("Failed to sequence withdrawal: {}", e);
            SequencerError::from(e)
        })?;

    // Send to batcher for batching if configured
    if let Some(batcher) = &state.batcher {
        if let Err(e) = batcher.add_message(cbor_message).await {
            warn!(
                sequence = receipt.sequence,
                error = %e,
                "Failed to add to batcher (sequencing succeeded)"
            );
        }
    }

    info!(
        sequence = receipt.sequence,
        request_id = %request.request_id,
        "Withdrawal request sequenced"
    );

    Ok((StatusCode::CREATED, Json(SequenceResponse::from(receipt))))
}

/// Receive and sequence a database snapshot (CBOR format)
async fn receive_snapshot(
    State(state): State<AppState>,
    Cbor(request): Cbor<SnapshotRequest>,
) -> Result<impl IntoResponse, HttpError> {
    info!(
        message_id = %request.message_id,
        snapshot_size = request.snapshot.data.len(),
        client_sequence = request.snapshot.sequence,
        "Received snapshot"
    );

    // Verify attestation token if verifier is configured
    if let Some(verifier) = &state.attestation_verifier {
        match &request.attestation_token {
            Some(token) => {
                verifier
                    .verify(token)
                    .await
                    .map_err(|e| {
                        error!(message_id = %request.message_id, error = %e, "Attestation verification failed");
                        SequencerError::from(e)
                    })?;
                info!(message_id = %request.message_id, "Attestation token verified");
            }
            None => {
                error!(message_id = %request.message_id, "Missing attestation token");
                return Err(SequencerError::MissingAttestationToken.into());
            }
        }
    }

    // Serialize the snapshot request as CBOR payload
    let payload = request.to_cbor().map_err(|e| {
        error!("Failed to serialize snapshot: {}", e);
        SequencerError::CborSerializationFailed(e.to_string())
    })?;

    // Sequence and sign the message (creates COSE signed message)
    let (cbor_message, receipt) = state
        .inbox
        .sequence_message(CborMessageType::Snapshot, payload)
        .map_err(|e| {
            error!("Failed to sequence snapshot: {}", e);
            SequencerError::from(e)
        })?;

    // Send to batcher for batching if configured
    if let Some(batcher) = &state.batcher {
        if let Err(e) = batcher.add_message(cbor_message).await {
            warn!(
                sequence = receipt.sequence,
                error = %e,
                "Failed to add to batcher (sequencing succeeded)"
            );
        }
    }

    info!(
        sequence = receipt.sequence,
        message_id = %request.message_id,
        client_sequence = request.snapshot.sequence,
        "Snapshot sequenced"
    );

    Ok((StatusCode::CREATED, Json(SequenceResponse::from(receipt))))
}

/// Health check endpoint (liveness probe)
///
/// Returns OK if the server is running. This is a simple liveness check
/// that doesn't verify external dependencies.
async fn health_check() -> &'static str {
    "OK"
}

/// Readiness check endpoint
///
/// Returns OK if the server is ready to accept traffic.
/// Checks that the batcher (if configured) is accessible.
async fn readiness_check(
    State(state): State<AppState>,
) -> Result<Json<ReadinessResponse>, HttpError> {
    let mut checks = Vec::new();

    // Check inbox is operational
    checks.push(HealthCheck {
        name: "inbox".to_string(),
        status: "ok".to_string(),
        message: None,
    });

    // Check batcher if configured
    if let Some(batcher) = &state.batcher {
        match batcher.stats().await {
            Ok(stats) => {
                checks.push(HealthCheck {
                    name: "batcher".to_string(),
                    status: "ok".to_string(),
                    message: Some(format!(
                        "batches={}, messages={}, pending={}",
                        stats.batches_published, stats.messages_published, stats.pending_messages
                    )),
                });
            }
            Err(e) => {
                checks.push(HealthCheck {
                    name: "batcher".to_string(),
                    status: "degraded".to_string(),
                    message: Some(format!("Error: {e}")),
                });
            }
        }
    } else {
        checks.push(HealthCheck {
            name: "batcher".to_string(),
            status: "not_configured".to_string(),
            message: None,
        });
    }

    // Overall status is "ready" if all checks pass
    let all_ok = checks
        .iter()
        .all(|c| c.status == "ok" || c.status == "not_configured");
    let status = if all_ok { "ready" } else { "degraded" };

    Ok(Json(ReadinessResponse {
        status: status.to_string(),
        checks,
    }))
}

/// Status endpoint - returns current sequence and signer info
async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let pubkey = state.inbox.key_manager().public_key();
    Json(StatusResponse {
        current_sequence: state.inbox.current_sequence(),
        signer_address: format!("{:?}", state.inbox.signer_address()),
        signer_pubkey: format!("0x{}", hex::encode(pubkey)),
    })
}

/// Get CBOR batch statistics
async fn batch_stats(State(state): State<AppState>) -> Json<BatchStatsResponse> {
    match &state.batcher {
        Some(batcher) => match batcher.stats().await {
            Ok(stats) => Json(BatchStatsResponse::from(stats)),
            Err(e) => {
                warn!(error = %e, "Failed to get batch stats");
                Json(BatchStatsResponse {
                    enabled: true,
                    ..Default::default()
                })
            }
        },
        None => Json(BatchStatsResponse {
            enabled: false,
            ..Default::default()
        }),
    }
}

/// Force flush the current batch
async fn batch_flush(State(state): State<AppState>) -> Result<Json<BatchFlushResponse>, HttpError> {
    let batcher = state.batcher.as_ref().ok_or(SequencerError::NoBatcher)?;

    match batcher.flush().await {
        Ok(Some(metadata)) => {
            info!(
                reference = %metadata.reference,
                compressed_bytes = metadata.compressed_bytes,
                "Manual batch flush completed"
            );
            Ok(Json(BatchFlushResponse {
                published: true,
                reference: Some(metadata.reference),
                message_count: None, // Could track this if needed
                compressed_bytes: Some(metadata.compressed_bytes),
            }))
        }
        Ok(None) => Ok(Json(BatchFlushResponse {
            published: false,
            reference: None,
            message_count: None,
            compressed_bytes: None,
        })),
        Err(e) => {
            error!(error = %e, "Failed to flush batch");
            Err(SequencerError::BatchFlushFailed(e.to_string()).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        response::Response,
    };
    use synddb_shared::{
        keys::EvmKeyManager,
        types::payloads::{ChangesetData, SnapshotData},
    };
    use tower::ServiceExt;

    fn test_app() -> Router {
        let key_manager = Arc::new(EvmKeyManager::generate());
        let inbox = Arc::new(Inbox::new(key_manager));
        let state = AppState {
            inbox,
            attestation_verifier: None,
            batcher: None,
        };
        create_router(state)
    }

    /// Send a CBOR-serialized request to the server
    async fn send_cbor<T: Serialize>(app: Router, data: &T, uri: &str) -> Response {
        let mut buf = Vec::new();
        ciborium::into_writer(data, &mut buf).unwrap();
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/cbor")
                .body(Body::from(buf))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_health_check() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_status() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: StatusResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(status.current_sequence, 0);
        assert!(status.signer_address.starts_with("0x"));
        // Public key should be 64 bytes (128 hex chars + 0x prefix)
        assert!(status.signer_pubkey.starts_with("0x"));
        assert_eq!(status.signer_pubkey.len(), 130);
    }

    #[tokio::test]
    async fn test_receive_changesets() {
        let app = test_app();

        let request = ChangesetBatchRequest {
            batch_id: "test-batch-1".to_string(),
            changesets: vec![ChangesetData {
                data: b"test data".to_vec(),
                sequence: 0,
                timestamp: 1704067200,
            }],
            attestation_token: None,
        };

        let response = send_cbor(app, &request, "/changesets").await;

        assert_eq!(response.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt: SequenceResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(receipt.sequence, 0);
        assert!(receipt.signature.starts_with("0x"));
        assert!(receipt.message_hash.starts_with("0x"));
    }

    #[tokio::test]
    async fn test_receive_withdrawal() {
        let app = test_app();

        let request = WithdrawalRequest {
            request_id: "withdrawal-1".to_string(),
            recipient: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            amount: "1000000000000000000".to_string(),
            data: vec![],
        };

        let response = send_cbor(app, &request, "/withdrawals").await;

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_withdrawal_invalid_address() {
        let app = test_app();

        let request = WithdrawalRequest {
            request_id: "withdrawal-1".to_string(),
            recipient: "invalid-address".to_string(),
            amount: "1000000000000000000".to_string(),
            data: vec![],
        };

        let response = send_cbor(app, &request, "/withdrawals").await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_sequence_increments() {
        let key_manager = Arc::new(EvmKeyManager::generate());
        let inbox = Arc::new(Inbox::new(key_manager));
        let state = AppState {
            inbox: inbox.clone(),
            attestation_verifier: None,
            batcher: None,
        };
        let app = create_router(state);

        // First request
        let request1 = ChangesetBatchRequest {
            batch_id: "batch-1".to_string(),
            changesets: vec![],
            attestation_token: None,
        };

        let response = send_cbor(app.clone(), &request1, "/changesets").await;

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt1: SequenceResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(receipt1.sequence, 0);

        // Second request
        let request2 = ChangesetBatchRequest {
            batch_id: "batch-2".to_string(),
            changesets: vec![],
            attestation_token: None,
        };

        let response = send_cbor(app, &request2, "/changesets").await;

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt2: SequenceResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(receipt2.sequence, 1);

        // Verify inbox state
        assert_eq!(inbox.current_sequence(), 2);
    }

    #[tokio::test]
    async fn test_withdrawal_with_valid_data() {
        let app = test_app();

        let request = WithdrawalRequest {
            request_id: "withdrawal-valid".to_string(),
            recipient: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            amount: "1000000000000000000".to_string(),
            data: b"Hello World".to_vec(),
        };

        let response = send_cbor(app, &request, "/withdrawals").await;

        assert_eq!(response.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt: SequenceResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(receipt.sequence, 0);
        assert!(receipt.signature.starts_with("0x"));
        assert!(receipt.signer.starts_with("0x"));
    }

    #[tokio::test]
    async fn test_withdrawal_invalid_hex_in_address() {
        let app = test_app();

        let request = WithdrawalRequest {
            request_id: "withdrawal-1".to_string(),
            recipient: "0xGGGG35Cc6634C0532925a3b844Bc454e4438f44e".to_string(), // Invalid hex
            amount: "1000000000000000000".to_string(),
            data: vec![],
        };

        let response = send_cbor(app, &request, "/withdrawals").await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_withdrawal_invalid_amount_format() {
        // Test empty amount
        let request1 = WithdrawalRequest {
            request_id: "withdrawal-1".to_string(),
            recipient: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            amount: String::new(),
            data: vec![],
        };

        let response = send_cbor(test_app(), &request1, "/withdrawals").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Test non-numeric amount
        let request2 = WithdrawalRequest {
            request_id: "withdrawal-2".to_string(),
            recipient: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            amount: "abc123".to_string(),
            data: vec![],
        };

        let response = send_cbor(test_app(), &request2, "/withdrawals").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Test leading zeros
        let request3 = WithdrawalRequest {
            request_id: "withdrawal-3".to_string(),
            recipient: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            amount: "0123".to_string(),
            data: vec![],
        };

        let response = send_cbor(test_app(), &request3, "/withdrawals").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_withdrawal_empty_request_id() {
        let app = test_app();

        let request = WithdrawalRequest {
            request_id: String::new(),
            recipient: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            amount: "1000".to_string(),
            data: vec![],
        };

        let response = send_cbor(app, &request, "/withdrawals").await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_withdrawal_zero_amount() {
        let app = test_app();

        // "0" should be valid (single zero is allowed)
        let request = WithdrawalRequest {
            request_id: "withdrawal-zero".to_string(),
            recipient: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            amount: "0".to_string(),
            data: vec![],
        };

        let response = send_cbor(app, &request, "/withdrawals").await;

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_readiness_no_batcher() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let readiness: ReadinessResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(readiness.status, "ready");
        assert_eq!(readiness.checks.len(), 2);
        assert!(readiness
            .checks
            .iter()
            .any(|c| c.name == "inbox" && c.status == "ok"));
        assert!(readiness
            .checks
            .iter()
            .any(|c| c.name == "batcher" && c.status == "not_configured"));
    }

    #[tokio::test]
    async fn test_receive_snapshot() {
        let app = test_app();

        let request = SnapshotRequest {
            message_id: "snapshot-test-1".to_string(),
            snapshot: SnapshotData {
                data: b"SQLite format 3\x00".to_vec(),
                timestamp: 1704067200,
                sequence: 100,
            },
            attestation_token: None,
        };

        let response = send_cbor(app, &request, "/snapshots").await;

        assert_eq!(response.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt: SequenceResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(receipt.sequence, 0);
        assert!(receipt.signature.starts_with("0x"));
        assert!(receipt.message_hash.starts_with("0x"));
    }

    #[tokio::test]
    async fn test_snapshot_sequence_independence() {
        let key_manager = Arc::new(EvmKeyManager::generate());
        let inbox = Arc::new(Inbox::new(key_manager));
        let state = AppState {
            inbox: inbox.clone(),
            attestation_verifier: None,
            batcher: None,
        };
        let app = create_router(state);

        // Send a snapshot with client sequence 100
        let request1 = SnapshotRequest {
            message_id: "snap-1".to_string(),
            snapshot: SnapshotData {
                data: b"data1".to_vec(),
                timestamp: 1704067200,
                sequence: 100,
            },
            attestation_token: None,
        };

        let response = send_cbor(app.clone(), &request1, "/snapshots").await;

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt1: SequenceResponse = serde_json::from_slice(&body).unwrap();

        // Send another snapshot with client sequence 200
        let request2 = SnapshotRequest {
            message_id: "snap-2".to_string(),
            snapshot: SnapshotData {
                data: b"data2".to_vec(),
                timestamp: 1704067300,
                sequence: 200,
            },
            attestation_token: None,
        };

        let response = send_cbor(app, &request2, "/snapshots").await;

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt2: SequenceResponse = serde_json::from_slice(&body).unwrap();

        // Sequencer assigns independent sequence numbers
        assert_eq!(receipt1.sequence, 0);
        assert_eq!(receipt2.sequence, 1);

        // Verify inbox sequence
        assert_eq!(inbox.current_sequence(), 2);
    }
}
