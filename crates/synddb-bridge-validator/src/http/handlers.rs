use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sha3::Digest;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    bridge::BridgeClient,
    config::ValidatorMode,
    signing::MessageSigner,
    storage::{
        record::{MessageRecord, PublicationRecord, SignatureRecord},
        StoragePublisher, StorageRecord,
    },
    types::{
        compute_message_id, compute_metadata_hash, ErrorResponse, Message, MessageRequest,
        MessageResponse, MessageStatus,
    },
    validation::ValidationPipeline,
};

pub struct AppState {
    pub mode: ValidatorMode,
    pub pipeline: Arc<ValidationPipeline>,
    pub signer: Arc<MessageSigner>,
    pub bridge_client: Arc<BridgeClient>,
    pub storage: Arc<dyn StoragePublisher>,
    pub api_key: Option<String>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("mode", &self.mode)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub healthy: bool,
    pub mode: String,
    pub synced: bool,
}

pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        healthy: true,
        mode: format!("{:?}", state.mode),
        synced: true,
    })
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub ready: bool,
}

pub async fn ready() -> Json<ReadyResponse> {
    Json(ReadyResponse { ready: true })
}

pub async fn submit_message(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<MessageResponse>, (StatusCode, Json<MessageResponse>)> {
    // 1. Parse request and compute message ID
    let metadata_hash = compute_metadata_hash(&request.metadata).map_err(|e| {
        error_response(
            StatusCode::BAD_REQUEST,
            "METADATA_INVALID",
            &format!("Failed to compute metadata hash: {}", e),
        )
    })?;

    let message_id = compute_message_id(
        &request.message_type,
        &request.calldata,
        &metadata_hash,
        request.nonce,
        request.timestamp,
        &request.domain,
    );

    let value = request
        .value
        .as_ref()
        .map(|v| v.parse::<u128>())
        .transpose()
        .map_err(|e| {
            error_response(
                StatusCode::BAD_REQUEST,
                "VALUE_INVALID",
                &format!("Failed to parse value: {}", e),
            )
        })?;

    let message = Message {
        id: message_id,
        message_type: request.message_type.clone(),
        calldata: request.calldata.clone(),
        metadata: request.metadata.clone(),
        metadata_hash,
        nonce: request.nonce,
        timestamp: request.timestamp,
        domain: request.domain,
        value,
    };

    // 2. Fetch validation context from bridge
    let ctx = state
        .pipeline
        .fetch_context(&message, &state.bridge_client)
        .await
        .map_err(|e| {
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                e.error_code(),
                &e.to_string(),
            )
        })?;

    // 3. Run validation pipeline
    state
        .pipeline
        .validate(&message, &ctx)
        .await
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, e.error_code(), &e.to_string()))?;

    // 4. Sign the message
    let signature = state.signer.sign_message(&message).await.map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SIGNING_FAILED",
            &format!("Failed to sign message: {}", e),
        )
    })?;

    // 5. Publish to storage
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let validator_address: [u8; 20] = state.signer.address().into_array();

    let record = StorageRecord {
        message: MessageRecord::from(&message),
        primary_signature: SignatureRecord {
            validator: validator_address,
            signature: signature.clone(),
            signed_at: now,
        },
        publication: PublicationRecord {
            published_by: validator_address,
            published_at: now,
        },
    };

    let storage_ref = state.storage.publish(&record).await.map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_PUBLISH_FAILED",
            &format!("Failed to publish to storage: {}", e),
        )
    })?;

    // 6. Initialize message on bridge
    state
        .bridge_client
        .initialize_message(&message, &storage_ref, value)
        .await
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "BRIDGE_SUBMIT_FAILED",
                &format!("Failed to initialize message on bridge: {}", e),
            )
        })?;

    // 7. Consume nonce and mark processed
    state
        .pipeline
        .consume_nonce(&message.domain, message.nonce)
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                &e.to_string(),
            )
        })?;

    state
        .pipeline
        .mark_message_processed(&message.id)
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                &e.to_string(),
            )
        })?;

    // 8. Return success response
    tracing::info!(
        message_id = %hex::encode(message_id),
        storage_ref = %storage_ref,
        "Message accepted and initialized on bridge"
    );

    Ok(Json(MessageResponse {
        status: MessageStatus::Accepted,
        message_id,
        signature: Some(signature),
        storage_ref: Some(storage_ref),
        error: None,
    }))
}

fn error_response(
    status: StatusCode,
    code: &str,
    message: &str,
) -> (StatusCode, Json<MessageResponse>) {
    tracing::warn!(code = code, message = message, "Message rejected");

    (
        status,
        Json(MessageResponse {
            status: MessageStatus::Rejected,
            message_id: [0u8; 32],
            signature: None,
            storage_ref: None,
            error: Some(ErrorResponse {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }),
    )
}

#[derive(Debug, Serialize)]
pub struct MessageStatusResponse {
    pub id: String,
    pub status: String,
    pub stage: u8,
    pub signatures_collected: u64,
    pub signature_threshold: u64,
    pub executed: bool,
}

pub async fn get_message_status(
    State(state): State<Arc<AppState>>,
    Path(message_id): Path<String>,
) -> Result<Json<MessageStatusResponse>, (StatusCode, String)> {
    // Parse message ID from hex
    let id_bytes: [u8; 32] = hex::decode(message_id.trim_start_matches("0x"))
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid message ID: {}", e),
            )
        })?
        .try_into()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Message ID must be 32 bytes".to_string(),
            )
        })?;

    // Query bridge for message stage
    let stage = state
        .bridge_client
        .get_message_stage(id_bytes)
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Failed to query bridge: {}", e),
            )
        })?;

    // Query signature count
    let signatures_collected = state
        .bridge_client
        .get_signature_count(id_bytes)
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Failed to query signatures: {}", e),
            )
        })?;

    // Query threshold
    let signature_threshold = state
        .bridge_client
        .get_signature_threshold()
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Failed to query threshold: {}", e),
            )
        })?;

    // Map stage to status string
    let status = match stage {
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
    };

    Ok(Json(MessageStatusResponse {
        id: format!("0x{}", hex::encode(id_bytes)),
        status: status.to_string(),
        stage,
        signatures_collected,
        signature_threshold,
        executed: stage == 6,
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SchemaResponse {
    pub message_type: String,
    pub schema: Option<serde_json::Value>,
    pub schema_hash: String,
    pub schema_uri: String,
    pub enabled: bool,
    pub target: String,
}

pub async fn get_schema(
    State(state): State<Arc<AppState>>,
    Path(message_type): Path<String>,
) -> Result<Json<SchemaResponse>, (StatusCode, String)> {
    // Fetch message type config from bridge
    let config = state
        .bridge_client
        .get_message_type_config(&message_type)
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Failed to query bridge: {}", e),
            )
        })?;

    // Try to fetch schema if URI is present
    let schema = if config.schema_uri.is_empty() {
        None
    } else {
        let expected_hash =
            (config.schema_hash != [0u8; 32]).then_some(&config.schema_hash);

        // Use a temporary fetcher since we don't have direct access to pipeline's
        use crate::validation::SchemaFetcher;
        use std::time::Duration;

        let fetcher = SchemaFetcher::new(Duration::from_secs(3600));
        match fetcher.fetch(&config.schema_uri, expected_hash).await {
            Ok(schema) => Some(schema),
            Err(e) => {
                tracing::warn!(
                    schema_uri = %config.schema_uri,
                    error = %e,
                    "Failed to fetch schema"
                );
                None
            }
        }
    };

    Ok(Json(SchemaResponse {
        message_type: config.message_type,
        schema,
        schema_hash: format!("0x{}", hex::encode(config.schema_hash)),
        schema_uri: config.schema_uri,
        enabled: config.enabled,
        target: format!("{}", config.target),
    }))
}

#[derive(Debug, Deserialize)]
pub struct RejectProposalRequest {
    pub message_type: String,
    #[serde(with = "crate::types::hex_bytes_32")]
    pub domain: [u8; 32],
    pub nonce: u64,
    pub reason: RejectionReason,
}

#[derive(Debug, Deserialize)]
pub struct RejectMessageRequest {
    #[serde(with = "crate::types::hex_bytes_32")]
    pub message_id: [u8; 32],
    pub reason: RejectionReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionReason {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct RejectionResponse {
    pub success: bool,
    pub message_id: String,
    pub reason_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Reject a message proposal before initialization (Primary Validator only)
/// This consumes the nonce, preventing the message from ever being initialized.
pub async fn reject_proposal(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RejectProposalRequest>,
) -> Result<Json<RejectionResponse>, (StatusCode, Json<RejectionResponse>)> {
    // Compute message ID from the proposal details
    let metadata_hash = [0u8; 32]; // Rejection doesn't require metadata
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let message_id = compute_message_id(
        &request.message_type,
        &[], // No calldata for rejection
        &metadata_hash,
        request.nonce,
        timestamp,
        &request.domain,
    );

    // Compute reason hash
    let reason_json = serde_json::to_vec(&request.reason).map_err(|e| {
        rejection_error_response(
            StatusCode::BAD_REQUEST,
            &message_id,
            &format!("Failed to serialize reason: {}", e),
        )
    })?;
    let reason_hash: [u8; 32] = sha3::Keccak256::digest(&reason_json).into();

    // Create a minimal message for the rejection
    let message = Message {
        id: message_id,
        message_type: request.message_type.clone(),
        calldata: vec![],
        metadata: serde_json::json!({}),
        metadata_hash,
        nonce: request.nonce,
        timestamp,
        domain: request.domain,
        value: None,
    };

    // Publish rejection reason to storage
    let reason_ref = format!("rejection:{}", hex::encode(reason_hash));

    // Call rejectProposal on bridge
    state
        .bridge_client
        .reject_proposal(&message, reason_hash, &reason_ref)
        .await
        .map_err(|e| {
            rejection_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &message_id,
                &format!("Failed to reject proposal on bridge: {}", e),
            )
        })?;

    // Consume nonce locally
    let _ = state.pipeline.consume_nonce(&request.domain, request.nonce);

    tracing::info!(
        message_id = %hex::encode(message_id),
        reason_code = %request.reason.code,
        "Proposal rejected"
    );

    Ok(Json(RejectionResponse {
        success: true,
        message_id: format!("0x{}", hex::encode(message_id)),
        reason_hash: format!("0x{}", hex::encode(reason_hash)),
        error: None,
    }))
}

/// Reject an initialized message (Witness Validator action)
/// This records the rejection but does not prevent execution if threshold is met.
pub async fn reject_message(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RejectMessageRequest>,
) -> Result<Json<RejectionResponse>, (StatusCode, Json<RejectionResponse>)> {
    // Compute reason hash
    let reason_json = serde_json::to_vec(&request.reason).map_err(|e| {
        rejection_error_response(
            StatusCode::BAD_REQUEST,
            &request.message_id,
            &format!("Failed to serialize reason: {}", e),
        )
    })?;
    let reason_hash: [u8; 32] = sha3::Keccak256::digest(&reason_json).into();

    let reason_ref = format!("rejection:{}", hex::encode(reason_hash));

    // Call rejectMessage on bridge
    state
        .bridge_client
        .reject_message(request.message_id, reason_hash, &reason_ref)
        .await
        .map_err(|e| {
            rejection_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &request.message_id,
                &format!("Failed to reject message on bridge: {}", e),
            )
        })?;

    tracing::info!(
        message_id = %hex::encode(request.message_id),
        reason_code = %request.reason.code,
        "Message rejected"
    );

    Ok(Json(RejectionResponse {
        success: true,
        message_id: format!("0x{}", hex::encode(request.message_id)),
        reason_hash: format!("0x{}", hex::encode(reason_hash)),
        error: None,
    }))
}

fn rejection_error_response(
    status: StatusCode,
    message_id: &[u8; 32],
    error_message: &str,
) -> (StatusCode, Json<RejectionResponse>) {
    tracing::warn!(
        message_id = %hex::encode(message_id),
        error = error_message,
        "Rejection failed"
    );

    (
        status,
        Json(RejectionResponse {
            success: false,
            message_id: format!("0x{}", hex::encode(message_id)),
            reason_hash: String::new(),
            error: Some(error_message.to_string()),
        }),
    )
}
