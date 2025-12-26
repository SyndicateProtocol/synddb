use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config::ValidatorMode;
use crate::types::{ErrorResponse, MessageRequest, MessageResponse, MessageStatus};

pub struct AppState {
    pub mode: ValidatorMode,
    // TODO: Add pipeline, signer, storage, bridge client
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
    State(_state): State<Arc<AppState>>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<MessageResponse>, (StatusCode, Json<MessageResponse>)> {
    // TODO: Implement full message validation flow
    // 1. Compute message ID
    // 2. Run validation pipeline
    // 3. Sign message
    // 4. Publish to storage
    // 5. Initialize on bridge
    // 6. Return response

    let message_id = [0u8; 32]; // TODO: compute actual message ID

    // Placeholder response
    let response = MessageResponse {
        status: MessageStatus::Rejected,
        message_id,
        signature: None,
        storage_ref: None,
        error: Some(ErrorResponse {
            code: "NOT_IMPLEMENTED".to_string(),
            message: format!("Message submission not yet implemented for type: {}", request.message_type),
        }),
    };

    Err((StatusCode::NOT_IMPLEMENTED, Json(response)))
}

#[derive(Debug, Serialize)]
pub struct MessageStatusResponse {
    pub id: String,
    pub status: String,
    pub signatures_collected: u32,
}

pub async fn get_message_status(
    State(_state): State<Arc<AppState>>,
    Path(message_id): Path<String>,
) -> Result<Json<MessageStatusResponse>, StatusCode> {
    // TODO: Query bridge for message status

    Ok(Json(MessageStatusResponse {
        id: message_id,
        status: "unknown".to_string(),
        signatures_collected: 0,
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SchemaResponse {
    pub message_type: String,
    pub schema: Option<serde_json::Value>,
    pub schema_hash: String,
    pub schema_uri: String,
}

pub async fn get_schema(
    State(_state): State<Arc<AppState>>,
    Path(message_type): Path<String>,
) -> Result<Json<SchemaResponse>, StatusCode> {
    // TODO: Fetch schema from cache or bridge

    Ok(Json(SchemaResponse {
        message_type,
        schema: None,
        schema_hash: "0x".to_string() + &hex::encode([0u8; 32]),
        schema_uri: String::new(),
    }))
}
