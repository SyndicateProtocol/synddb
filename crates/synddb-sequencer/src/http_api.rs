//! HTTP API for the sequencer node
//!
//! Provides endpoints for:
//! - Receiving changeset batches from synddb-client
//! - Receiving withdrawal requests
//! - Health and status checks

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use crate::attestation::AttestationVerifier;
use crate::inbox::{Inbox, MessageType, SequenceReceipt, SignedMessage};
use crate::publish::DAPublisher;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub inbox: Arc<Inbox>,
    /// Optional publisher for persisting messages
    pub publisher: Option<Arc<dyn DAPublisher>>,
    /// Optional attestation verifier for TEE token validation
    pub attestation_verifier: Option<Arc<AttestationVerifier>>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("inbox", &self.inbox)
            .field("publisher", &self.publisher.is_some())
            .field("attestation_verifier", &self.attestation_verifier.is_some())
            .finish()
    }
}

/// Create the HTTP router with all endpoints
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/changesets", post(receive_changesets))
        .route("/withdrawals", post(receive_withdrawal))
        .route("/messages/:sequence", get(get_message))
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .route("/status", get(status))
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Changeset data from synddb-client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetData {
    /// Raw changeset bytes (base64 encoded in JSON)
    #[serde(with = "base64_serde")]
    pub data: Vec<u8>,
    /// Client-side sequence number
    pub sequence: u64,
    /// Client-side timestamp (Unix timestamp in seconds)
    pub timestamp: u64,
}

/// Changeset batch request from synddb-client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetBatchRequest {
    /// Batch identifier for tracking
    pub batch_id: String,
    /// List of changesets in this batch
    pub changesets: Vec<ChangesetData>,
    /// Optional TEE attestation token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_token: Option<String>,
}

/// Withdrawal request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawalRequest {
    /// Unique request identifier
    pub request_id: String,
    /// Recipient address (Ethereum format)
    pub recipient: String,
    /// Amount to withdraw (as string to handle large numbers)
    pub amount: String,
    /// Optional calldata
    #[serde(default, with = "base64_serde")]
    pub data: Vec<u8>,
}

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
    /// Sequencer's Ethereum address
    pub signer_address: String,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
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

/// Response for message retrieval
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    /// Sequence number
    pub sequence: u64,
    /// Timestamp when sequenced
    pub timestamp: u64,
    /// Message type
    pub message_type: String,
    /// Original payload (base64 encoded)
    #[serde(with = "base64_serde")]
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

/// Receive and sequence a changeset batch
async fn receive_changesets(
    State(state): State<AppState>,
    Json(request): Json<ChangesetBatchRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    info!(
        batch_id = %request.batch_id,
        changeset_count = request.changesets.len(),
        "Received changeset batch"
    );

    // Verify attestation token if verifier is configured
    if let Some(verifier) = &state.attestation_verifier {
        match &request.attestation_token {
            Some(token) => {
                verifier.verify(token).await.map_err(|e| {
                    warn!(batch_id = %request.batch_id, error = %e, "Attestation verification failed");
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(ErrorResponse {
                            error: format!("Attestation verification failed: {e}"),
                        }),
                    )
                })?;
                info!(batch_id = %request.batch_id, "Attestation token verified");
            }
            None => {
                warn!(batch_id = %request.batch_id, "Missing attestation token");
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse {
                        error: "Attestation token required but not provided".to_string(),
                    }),
                ));
            }
        }
    }

    // Serialize the batch as the payload
    let payload = serde_json::to_vec(&request).map_err(|e| {
        warn!("Failed to serialize changeset batch: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Serialization failed".to_string(),
            }),
        )
    })?;

    // Sequence and sign the message
    let (signed_message, receipt) = state
        .inbox
        .sequence_message(MessageType::Changeset, payload)
        .await
        .map_err(|e| {
            warn!("Failed to sequence message: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Signing failed: {e}"),
                }),
            )
        })?;

    // Publish to DA layer if configured
    if let Some(publisher) = &state.publisher {
        let publish_result = publisher.publish(&signed_message).await;
        if !publish_result.success {
            warn!(
                sequence = receipt.sequence,
                error = ?publish_result.error,
                "Failed to publish message (sequencing succeeded)"
            );
            // Note: We still return success since the message was sequenced
            // The publish failure should be retried asynchronously
        }
    }

    info!(
        sequence = receipt.sequence,
        batch_id = %request.batch_id,
        "Changeset batch sequenced"
    );

    Ok((StatusCode::CREATED, Json(SequenceResponse::from(receipt))))
}

/// Receive and sequence a withdrawal request
async fn receive_withdrawal(
    State(state): State<AppState>,
    Json(request): Json<WithdrawalRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
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
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid recipient address format: must be 0x followed by 40 hex characters"
                    .to_string(),
            }),
        ));
    }

    // Validate amount is a valid decimal number (can be parsed as u128 or larger)
    if request.amount.is_empty()
        || !request.amount.chars().all(|c| c.is_ascii_digit())
        || (request.amount.len() > 1 && request.amount.starts_with('0'))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid amount format: must be a non-negative decimal number without leading zeros".to_string(),
            }),
        ));
    }

    // Validate request_id is not empty
    if request.request_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "request_id cannot be empty".to_string(),
            }),
        ));
    }

    // Serialize the request as the payload
    let payload = serde_json::to_vec(&request).map_err(|e| {
        warn!("Failed to serialize withdrawal request: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Serialization failed".to_string(),
            }),
        )
    })?;

    // Sequence and sign the message
    let (signed_message, receipt) = state
        .inbox
        .sequence_message(MessageType::Withdrawal, payload)
        .await
        .map_err(|e| {
            warn!("Failed to sequence withdrawal: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Signing failed: {e}"),
                }),
            )
        })?;

    // Publish to DA layer if configured
    if let Some(publisher) = &state.publisher {
        let publish_result = publisher.publish(&signed_message).await;
        if !publish_result.success {
            warn!(
                sequence = receipt.sequence,
                error = ?publish_result.error,
                "Failed to publish withdrawal (sequencing succeeded)"
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
/// Checks that the publisher (if configured) is accessible.
async fn readiness_check(
    State(state): State<AppState>,
) -> Result<Json<ReadinessResponse>, (StatusCode, Json<ErrorResponse>)> {
    let mut checks = Vec::new();

    // Check inbox is operational
    checks.push(HealthCheck {
        name: "inbox".to_string(),
        status: "ok".to_string(),
        message: None,
    });

    // Check publisher if configured
    if let Some(publisher) = &state.publisher {
        match publisher.get_latest_sequence().await {
            Ok(_) => {
                checks.push(HealthCheck {
                    name: "publisher".to_string(),
                    status: "ok".to_string(),
                    message: Some(format!("Connected to {}", publisher.name())),
                });
            }
            Err(e) => {
                checks.push(HealthCheck {
                    name: "publisher".to_string(),
                    status: "degraded".to_string(),
                    message: Some(format!("Error: {e}")),
                });
            }
        }
    } else {
        checks.push(HealthCheck {
            name: "publisher".to_string(),
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
    Json(StatusResponse {
        current_sequence: state.inbox.current_sequence(),
        signer_address: format!("{:?}", state.inbox.signer_address()),
    })
}

/// Get a message by sequence number
async fn get_message(
    State(state): State<AppState>,
    Path(sequence): Path<u64>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let publisher = state.publisher.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "No publisher configured - message retrieval unavailable".to_string(),
            }),
        )
    })?;

    match publisher.get(sequence).await {
        Ok(Some(message)) => Ok(Json(MessageResponse::from(message))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Message with sequence {} not found", sequence),
            }),
        )),
        Err(e) => {
            warn!(sequence, error = %e, "Failed to retrieve message");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to retrieve message: {e}"),
                }),
            ))
        }
    }
}

// ============================================================================
// Base64 Serde Helper
// ============================================================================

mod base64_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        encoded.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use base64::Engine;
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::MockPublisher;
    use crate::signer::MessageSigner;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn test_app() -> Router {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let inbox = Arc::new(Inbox::new(signer));
        let state = AppState {
            inbox,
            publisher: None,
            attestation_verifier: None,
        };
        create_router(state)
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
    }

    #[tokio::test]
    async fn test_receive_changesets() {
        let app = test_app();

        let request_body = serde_json::json!({
            "batch_id": "test-batch-1",
            "changesets": [
                {
                    "data": "dGVzdCBkYXRh",  // "test data" in base64
                    "sequence": 0,
                    "timestamp": 1704067200  // Unix timestamp
                }
            ]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/changesets")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

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

        let request_body = serde_json::json!({
            "request_id": "withdrawal-1",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "amount": "1000000000000000000",
            "data": ""
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_withdrawal_invalid_address() {
        let app = test_app();

        let request_body = serde_json::json!({
            "request_id": "withdrawal-1",
            "recipient": "invalid-address",
            "amount": "1000000000000000000"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_sequence_increments() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let inbox = Arc::new(Inbox::new(signer));
        let state = AppState {
            inbox: inbox.clone(),
            publisher: None,
            attestation_verifier: None,
        };
        let app = create_router(state);

        // First request
        let request_body = serde_json::json!({
            "batch_id": "batch-1",
            "changesets": []
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/changesets")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt1: SequenceResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(receipt1.sequence, 0);

        // Second request
        let request_body = serde_json::json!({
            "batch_id": "batch-2",
            "changesets": []
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/changesets")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let receipt2: SequenceResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(receipt2.sequence, 1);

        // Verify inbox state
        assert_eq!(inbox.current_sequence(), 2);
    }

    #[tokio::test]
    async fn test_receive_changesets_with_publisher() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let inbox = Arc::new(Inbox::new(signer));
        let publisher = Arc::new(MockPublisher::new());

        let state = AppState {
            inbox: inbox.clone(),
            publisher: Some(publisher.clone()),
            attestation_verifier: None,
        };
        let app = create_router(state);

        let request_body = serde_json::json!({
            "batch_id": "test-with-publisher",
            "changesets": []
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/changesets")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        // Verify message was published
        let published = publisher.get(0).await.unwrap();
        assert!(published.is_some());
        assert_eq!(published.unwrap().sequence, 0);
    }

    #[tokio::test]
    async fn test_withdrawal_with_valid_data() {
        let app = test_app();

        let request_body = serde_json::json!({
            "request_id": "withdrawal-valid",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "amount": "1000000000000000000",
            "data": "SGVsbG8gV29ybGQ="  // "Hello World" in base64
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

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

        let request_body = serde_json::json!({
            "request_id": "withdrawal-1",
            "recipient": "0xGGGG35Cc6634C0532925a3b844Bc454e4438f44e",  // Invalid hex
            "amount": "1000000000000000000"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_withdrawal_invalid_amount_format() {
        let app = test_app();

        // Test empty amount
        let request_body = serde_json::json!({
            "request_id": "withdrawal-1",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "amount": ""
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Test non-numeric amount
        let request_body = serde_json::json!({
            "request_id": "withdrawal-2",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "amount": "abc123"
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Test leading zeros
        let request_body = serde_json::json!({
            "request_id": "withdrawal-3",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "amount": "0123"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_withdrawal_empty_request_id() {
        let app = test_app();

        let request_body = serde_json::json!({
            "request_id": "",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "amount": "1000"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_withdrawal_zero_amount() {
        let app = test_app();

        // "0" should be valid (single zero is allowed)
        let request_body = serde_json::json!({
            "request_id": "withdrawal-zero",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "amount": "0"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/withdrawals")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_get_message_with_publisher() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let inbox = Arc::new(Inbox::new(signer));
        let publisher = Arc::new(MockPublisher::new());

        let state = AppState {
            inbox: inbox.clone(),
            publisher: Some(publisher.clone()),
            attestation_verifier: None,
        };
        let app = create_router(state);

        // First, create a message
        let request_body = serde_json::json!({
            "batch_id": "test-get-message",
            "changesets": []
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/changesets")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        // Now retrieve the message
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/messages/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let message: MessageResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(message.sequence, 0);
        assert_eq!(message.message_type, "changeset");
        assert!(message.signature.starts_with("0x"));
    }

    #[tokio::test]
    async fn test_get_message_not_found() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let inbox = Arc::new(Inbox::new(signer));
        let publisher = Arc::new(MockPublisher::new());

        let state = AppState {
            inbox,
            publisher: Some(publisher),
            attestation_verifier: None,
        };
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/messages/999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_message_no_publisher() {
        let app = test_app(); // No publisher configured

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/messages/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn test_readiness_no_publisher() {
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
            .any(|c| c.name == "publisher" && c.status == "not_configured"));
    }

    #[tokio::test]
    async fn test_readiness_with_publisher() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let inbox = Arc::new(Inbox::new(signer));
        let publisher = Arc::new(MockPublisher::new());

        let state = AppState {
            inbox,
            publisher: Some(publisher),
            attestation_verifier: None,
        };
        let app = create_router(state);

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
        assert!(readiness
            .checks
            .iter()
            .any(|c| c.name == "publisher" && c.status == "ok"));
    }
}
