//! HTTP API for message passing
//!
//! Provides endpoints for:
//! - Getting inbound messages (blockchain -> app)
//! - Acknowledging inbound messages
//! - Checking outbound message status (app -> blockchain)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

use super::{
    outbound::{OutboundMonitorHandle, OutboundStats, TrackedOutboundMessage},
    queue::{InboundMessage, MessageQueue, QueueStats},
};

/// Shared state for message API
#[derive(Clone)]
pub struct MessageApiState {
    pub queue: Arc<RwLock<MessageQueue>>,
    pub outbound: Option<OutboundMonitorHandle>,
}

impl std::fmt::Debug for MessageApiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageApiState")
            .field("queue", &"<MessageQueue>")
            .field("outbound", &self.outbound.is_some())
            .finish()
    }
}

impl MessageApiState {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(RwLock::new(MessageQueue::new())),
            outbound: None,
        }
    }

    pub fn with_queue(queue: MessageQueue) -> Self {
        Self {
            queue: Arc::new(RwLock::new(queue)),
            outbound: None,
        }
    }

    pub fn with_outbound(mut self, handle: OutboundMonitorHandle) -> Self {
        self.outbound = Some(handle);
        self
    }
}

impl Default for MessageApiState {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the messages router
pub fn create_messages_router(state: MessageApiState) -> Router {
    info!("Message API Endpoints:");
    info!("  GET  /messages/inbound       - Get pending inbound messages");
    info!("  POST /messages/inbound       - Push a new inbound message");
    info!("  POST /messages/inbound/:id/ack - Acknowledge a message");
    info!("  GET  /messages/inbound/:id   - Get a specific message");
    info!("  GET  /messages/inbound/stats - Get queue statistics");
    info!("  GET  /messages/outbound/:id/status - Get outbound message status");
    info!("  GET  /messages/outbound/stats - Get outbound message statistics");

    Router::new()
        .route(
            "/messages/inbound",
            get(get_inbound_messages).post(push_inbound_message),
        )
        .route("/messages/inbound/stats", get(get_queue_stats))
        .route("/messages/inbound/{id}", get(get_message_by_id))
        .route("/messages/inbound/{id}/ack", post(acknowledge_message))
        .route("/messages/outbound/{id}/status", get(get_outbound_status))
        .route("/messages/outbound/stats", get(get_outbound_stats))
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Query parameters for getting inbound messages
#[derive(Debug, Deserialize)]
pub struct GetMessagesQuery {
    /// Get messages with ID greater than this value
    #[serde(default)]
    pub after_id: u64,
    /// Maximum number of messages to return (default: 100, max: 1000)
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Filter by message type (optional)
    #[serde(rename = "type")]
    pub message_type: Option<String>,
    /// Only return pending (unacknowledged) messages
    #[serde(default = "default_pending_only")]
    pub pending_only: bool,
}

const fn default_limit() -> usize {
    100
}

const fn default_pending_only() -> bool {
    true
}

/// Response containing inbound messages
#[derive(Debug, Serialize, Deserialize)]
pub struct GetMessagesResponse {
    /// List of messages
    pub messages: Vec<InboundMessageResponse>,
    /// Whether there are more messages after these
    pub has_more: bool,
    /// Latest message ID in the queue
    pub latest_id: u64,
}

/// Individual message in response
#[derive(Debug, Serialize, Deserialize)]
pub struct InboundMessageResponse {
    /// Sequencer-assigned ID
    pub id: u64,
    /// Message ID from blockchain event
    pub message_id: String,
    /// Type of message
    #[serde(rename = "type")]
    pub message_type: String,
    /// JSON payload
    pub payload: serde_json::Value,
    /// Sender address
    pub sender: String,
    /// Transaction hash
    pub tx_hash: String,
    /// Block number
    pub block_number: u64,
    /// Confirmations
    pub confirmations: u64,
    /// Timestamp when captured
    pub timestamp: u64,
    /// Whether acknowledged
    pub acknowledged: bool,
}

impl From<&InboundMessage> for InboundMessageResponse {
    fn from(msg: &InboundMessage) -> Self {
        // Parse payload as JSON, fallback to string if invalid
        let payload = serde_json::from_str(&msg.payload)
            .unwrap_or_else(|_| serde_json::Value::String(msg.payload.clone()));

        Self {
            id: msg.id,
            message_id: msg.message_id.clone(),
            message_type: msg.message_type.clone(),
            payload,
            sender: msg.sender.clone(),
            tx_hash: msg.tx_hash.clone(),
            block_number: msg.block_number,
            confirmations: msg.confirmations,
            timestamp: msg.timestamp,
            acknowledged: msg.acknowledged,
        }
    }
}

/// Request to acknowledge a message
#[derive(Debug, Deserialize)]
pub struct AckRequest {
    /// Whether the message was successfully processed
    #[serde(default = "default_processed")]
    pub processed: bool,
    /// Optional note about processing
    pub note: Option<String>,
}

const fn default_processed() -> bool {
    true
}

/// Response for acknowledgment
#[derive(Debug, Serialize, Deserialize)]
pub struct AckResponse {
    /// Whether the message was found and acknowledged
    pub acknowledged: bool,
    /// Message ID that was acknowledged
    pub message_id: u64,
}

/// Response for outbound message status
#[derive(Debug, Serialize, Deserialize)]
pub struct OutboundStatusResponse {
    /// Message ID from app's `message_log`
    pub id: u64,
    /// Type of message
    pub message_type: String,
    /// Current status
    pub status: String,
    /// Transaction hash (if submitted)
    pub tx_hash: Option<String>,
    /// Confirmations (if submitted)
    pub confirmations: Option<u64>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// When message was first seen
    pub first_seen_at: u64,
    /// Last update timestamp
    pub updated_at: u64,
}

impl From<TrackedOutboundMessage> for OutboundStatusResponse {
    fn from(msg: TrackedOutboundMessage) -> Self {
        Self {
            id: msg.id,
            message_type: msg.message_type,
            status: msg.status.to_string(),
            tx_hash: msg.tx_hash,
            confirmations: msg.confirmations,
            error: msg.error,
            first_seen_at: msg.first_seen_at,
            updated_at: msg.updated_at,
        }
    }
}

/// Response for outbound statistics
#[derive(Debug, Serialize, Deserialize)]
pub struct OutboundStatsResponse {
    pub total: usize,
    pub pending: usize,
    pub queued: usize,
    pub submitting: usize,
    pub submitted: usize,
    pub confirmed: usize,
    pub failed: usize,
    /// Whether the outbound monitor is active
    pub monitor_active: bool,
}

impl From<OutboundStats> for OutboundStatsResponse {
    fn from(stats: OutboundStats) -> Self {
        Self {
            total: stats.total,
            pending: stats.pending,
            queued: stats.queued,
            submitting: stats.submitting,
            submitted: stats.submitted,
            confirmed: stats.confirmed,
            failed: stats.failed,
            monitor_active: true,
        }
    }
}

/// Queue statistics response
#[derive(Debug, Serialize, Deserialize)]
pub struct QueueStatsResponse {
    /// Total messages in queue
    pub total: usize,
    /// Pending (unacknowledged) messages
    pub pending: usize,
    /// Acknowledged messages
    pub acknowledged: usize,
    /// Maximum queue size
    pub max_size: usize,
}

impl From<QueueStats> for QueueStatsResponse {
    fn from(stats: QueueStats) -> Self {
        Self {
            total: stats.total,
            pending: stats.pending,
            acknowledged: stats.acknowledged,
            max_size: stats.max_size,
        }
    }
}

/// Request to push a new inbound message
#[derive(Debug, Deserialize)]
pub struct PushInboundRequest {
    /// Message ID from blockchain event (e.g., requestId)
    pub message_id: String,
    /// Type of message (e.g., `price_request`, `deposit`)
    #[serde(rename = "type")]
    pub message_type: String,
    /// JSON payload with message-specific data
    pub payload: serde_json::Value,
    /// Sender address on blockchain
    pub sender: String,
    /// Transaction hash where event was emitted
    pub tx_hash: String,
    /// Block number where event was emitted
    pub block_number: u64,
    /// Number of confirmations (optional, default: 0)
    #[serde(default)]
    pub confirmations: u64,
}

/// Response for pushing a new inbound message
#[derive(Debug, Serialize, Deserialize)]
pub struct PushInboundResponse {
    /// Sequencer-assigned ID for the message
    pub id: u64,
    /// The `message_id` that was provided
    pub message_id: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// Get inbound messages
///
/// Returns pending messages by default. Use query params to filter:
/// - `after_id`: Get messages after this ID
/// - `limit`: Maximum messages to return (default: 100)
/// - `type`: Filter by message type
/// - `pending_only`: Only unacknowledged (default: true)
async fn get_inbound_messages(
    State(state): State<MessageApiState>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<Json<GetMessagesResponse>, (StatusCode, String)> {
    let limit = query.limit.min(1000);

    let queue = state.queue.read().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {e}"),
        )
    })?;

    let messages: Vec<InboundMessageResponse> = if query.pending_only {
        // Get pending messages
        query.message_type.as_ref().map_or_else(
            || {
                queue
                    .get_pending_messages(limit + 1) // +1 to check has_more
                    .into_iter()
                    .filter(|m| m.id > query.after_id)
                    .take(limit)
                    .map(InboundMessageResponse::from)
                    .collect()
            },
            |msg_type| {
                queue
                    .get_pending_by_type(msg_type, limit)
                    .into_iter()
                    .filter(|m| m.id > query.after_id)
                    .map(InboundMessageResponse::from)
                    .collect()
            },
        )
    } else {
        // Get all messages after ID
        queue
            .get_messages_after(query.after_id, limit + 1)
            .into_iter()
            .filter(|m| {
                query
                    .message_type
                    .as_ref()
                    .is_none_or(|t| &m.message_type == t)
            })
            .take(limit)
            .map(InboundMessageResponse::from)
            .collect()
    };

    let has_more = messages.len() > limit;
    let messages: Vec<_> = messages.into_iter().take(limit).collect();
    let latest_id = queue.latest_id();

    Ok(Json(GetMessagesResponse {
        messages,
        has_more,
        latest_id,
    }))
}

/// Get a specific message by ID
async fn get_message_by_id(
    State(state): State<MessageApiState>,
    Path(id): Path<u64>,
) -> Result<Json<InboundMessageResponse>, (StatusCode, String)> {
    let queue = state.queue.read().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {e}"),
        )
    })?;

    queue
        .get_by_id(id)
        .map(InboundMessageResponse::from)
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Message {id} not found")))
}

/// Acknowledge an inbound message
///
/// Called when the application has processed a message. This marks
/// the message as handled so it won't be returned in future queries.
async fn acknowledge_message(
    State(state): State<MessageApiState>,
    Path(id): Path<u64>,
    Json(request): Json<AckRequest>,
) -> Result<Json<AckResponse>, (StatusCode, String)> {
    let mut queue = state.queue.write().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {e}"),
        )
    })?;

    let acknowledged = queue.acknowledge(id);

    if acknowledged {
        info!(
            message_id = id,
            processed = request.processed,
            note = ?request.note,
            "Message acknowledged"
        );
    } else {
        warn!(message_id = id, "Message not found or already acknowledged");
    }

    Ok(Json(AckResponse {
        acknowledged,
        message_id: id,
    }))
}

/// Get queue statistics
async fn get_queue_stats(
    State(state): State<MessageApiState>,
) -> Result<Json<QueueStatsResponse>, (StatusCode, String)> {
    let queue = state.queue.read().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {e}"),
        )
    })?;

    Ok(Json(QueueStatsResponse::from(queue.stats())))
}

/// Push a new inbound message
///
/// Called by chain monitors (or other sources) to add an inbound message
/// to the queue. The message will be assigned a sequence ID and made
/// available for apps to retrieve via GET /messages/inbound.
async fn push_inbound_message(
    State(state): State<MessageApiState>,
    Json(request): Json<PushInboundRequest>,
) -> Result<Json<PushInboundResponse>, (StatusCode, String)> {
    let payload_str = serde_json::to_string(&request.payload).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid payload JSON: {e}"),
        )
    })?;

    let mut queue = state.queue.write().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {e}"),
        )
    })?;

    let id = queue.add_message(
        request.message_id.clone(),
        request.message_type.clone(),
        payload_str,
        request.sender,
        request.tx_hash.clone(),
        request.block_number,
        request.confirmations,
    );

    info!(
        id = id,
        message_id = %request.message_id,
        message_type = %request.message_type,
        block_number = request.block_number,
        tx_hash = %request.tx_hash,
        "Inbound message pushed to queue"
    );

    Ok(Json(PushInboundResponse {
        id,
        message_id: request.message_id,
    }))
}

/// Get outbound message status
///
/// Checks the status of an outbound message (from app's `message_log`).
/// This is used by clients to track whether their messages were submitted
/// to the blockchain.
async fn get_outbound_status(
    State(state): State<MessageApiState>,
    Path(id): Path<u64>,
) -> Result<Json<OutboundStatusResponse>, (StatusCode, String)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Check the outbound tracker if available
    if let Some(ref outbound) = state.outbound {
        if let Some(msg) = outbound.get_status(id) {
            return Ok(Json(OutboundStatusResponse::from(msg)));
        }
    }

    // Message not found - return unknown status
    Ok(Json(OutboundStatusResponse {
        id,
        message_type: "unknown".to_string(),
        status: "unknown".to_string(),
        tx_hash: None,
        confirmations: None,
        error: if state.outbound.is_none() {
            Some("Outbound monitor not configured".to_string())
        } else {
            Some("Message not found".to_string())
        },
        first_seen_at: 0,
        updated_at: now,
    }))
}

/// Get outbound message statistics
async fn get_outbound_stats(
    State(state): State<MessageApiState>,
) -> Result<Json<OutboundStatsResponse>, (StatusCode, String)> {
    Ok(Json(state.outbound.as_ref().map_or(
        // Return empty stats if monitor not configured
        OutboundStatsResponse {
            total: 0,
            pending: 0,
            queued: 0,
            submitting: 0,
            submitted: 0,
            confirmed: 0,
            failed: 0,
            monitor_active: false,
        },
        |outbound| OutboundStatsResponse::from(outbound.stats()),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    fn test_app() -> Router {
        let state = MessageApiState::new();

        // Pre-populate with test messages
        {
            let mut queue = state.queue.write().unwrap();
            queue.add_message(
                "req-1".to_string(),
                "price_request".to_string(),
                r#"{"asset":"BTC","max_age":300}"#.to_string(),
                "0x1234567890abcdef".to_string(),
                "0xabc123".to_string(),
                100,
                12,
            );
            queue.add_message(
                "req-2".to_string(),
                "deposit".to_string(),
                r#"{"amount":"1000000"}"#.to_string(),
                "0xfedcba0987654321".to_string(),
                "0xdef456".to_string(),
                101,
                12,
            );
        }

        create_messages_router(state)
    }

    #[tokio::test]
    async fn test_get_inbound_messages() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/messages/inbound")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: GetMessagesResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(result.messages.len(), 2);
        assert!(!result.has_more);
    }

    #[tokio::test]
    async fn test_get_messages_by_type() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/messages/inbound?type=price_request")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: GetMessagesResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].message_type, "price_request");
    }

    #[tokio::test]
    async fn test_get_message_by_id() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/messages/inbound/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let msg: InboundMessageResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(msg.id, 1);
        assert_eq!(msg.message_type, "price_request");
    }

    #[tokio::test]
    async fn test_get_message_not_found() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/messages/inbound/999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_acknowledge_message() {
        let state = MessageApiState::new();
        {
            let mut queue = state.queue.write().unwrap();
            queue.add_message(
                "req-1".to_string(),
                "test".to_string(),
                "{}".to_string(),
                "0x".to_string(),
                "0x".to_string(),
                100,
                0,
            );
        }

        let app = create_messages_router(state.clone());

        // Acknowledge the message
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/messages/inbound/1/ack")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"processed": true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: AckResponse = serde_json::from_slice(&body).unwrap();

        assert!(result.acknowledged);
        assert_eq!(result.message_id, 1);

        // Verify it's no longer pending
        let queue = state.queue.read().unwrap();
        let pending = queue.get_pending_messages(10);
        assert_eq!(pending.len(), 0);
    }

    #[tokio::test]
    async fn test_queue_stats() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/messages/inbound/stats")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let stats: QueueStatsResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(stats.total, 2);
        assert_eq!(stats.pending, 2);
        assert_eq!(stats.acknowledged, 0);
    }

    #[tokio::test]
    async fn test_push_inbound_message() {
        let state = MessageApiState::new();
        let app = create_messages_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/messages/inbound")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        r#"{
                        "message_id": "0xabc123",
                        "type": "price_request",
                        "payload": {"asset": "BTC", "max_age": 300},
                        "sender": "0x1234567890abcdef",
                        "tx_hash": "0xdef456",
                        "block_number": 12345,
                        "confirmations": 6
                    }"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: PushInboundResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(result.id, 1);
        assert_eq!(result.message_id, "0xabc123");

        // Verify the message is in the queue
        let queue = state.queue.read().unwrap();
        let msg = queue.get_by_id(1).unwrap();
        assert_eq!(msg.message_type, "price_request");
        assert_eq!(msg.block_number, 12345);
        assert!(!msg.acknowledged);
    }
}
