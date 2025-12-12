//! HTTP API endpoints for bridge signature retrieval
//!
//! These endpoints are used by relayers to fetch validator signatures
//! and submit them to the bridge contract.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::bridge::signature_store::SignatureStore;

/// Response for pending signatures list
#[derive(Debug, Serialize, Deserialize)]
struct PendingResponse {
    count: usize,
    message_ids: Vec<String>,
}

/// Response for signer info
#[derive(Debug, Serialize, Deserialize)]
struct SignerInfoResponse {
    signer: String,
    bridge_contract: String,
    chain_id: u64,
}

/// State for signature API endpoints
#[derive(Clone, Debug)]
pub struct SignatureApiState {
    store: SignatureStore,
    signer_address: String,
    bridge_contract: String,
    chain_id: u64,
}

impl SignatureApiState {
    /// Create new signature API state
    #[allow(clippy::missing_const_for_fn)] // SignatureStore contains Arc
    pub fn new(
        store: SignatureStore,
        signer_address: String,
        bridge_contract: String,
        chain_id: u64,
    ) -> Self {
        Self {
            store,
            signer_address,
            bridge_contract,
            chain_id,
        }
    }
}

/// Create the signature API router
///
/// Endpoints:
/// - `GET /health` - Health check
/// - `GET /info` - Signer information
/// - `GET /pending` - List pending message IDs
/// - `GET /signature/{message_id}` - Get signature for a message
/// - `GET /signatures` - Get all pending signatures
pub fn create_signature_router(state: SignatureApiState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/info", get(info_handler))
        .route("/pending", get(pending_handler))
        .route("/signature/{message_id}", get(get_signature_handler))
        .route("/signatures", get(list_signatures_handler))
        .with_state(state)
}

/// Health check for signature API
async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy"
    }))
}

/// Get signer information
async fn info_handler(State(state): State<SignatureApiState>) -> impl IntoResponse {
    Json(SignerInfoResponse {
        signer: state.signer_address,
        bridge_contract: state.bridge_contract,
        chain_id: state.chain_id,
    })
}

/// Get list of pending message IDs
async fn pending_handler(State(state): State<SignatureApiState>) -> impl IntoResponse {
    let message_ids = state.store.pending_ids();
    Json(PendingResponse {
        count: message_ids.len(),
        message_ids,
    })
}

/// Get signature for a specific message ID
async fn get_signature_handler(
    State(state): State<SignatureApiState>,
    Path(message_id): Path<String>,
) -> impl IntoResponse {
    // Normalize the message ID (add 0x prefix if missing)
    let normalized_id = if message_id.starts_with("0x") {
        message_id
    } else {
        format!("0x{message_id}")
    };

    state
        .store
        .get(&normalized_id)
        .map_or((StatusCode::NOT_FOUND, Json(None)), |sig| {
            (StatusCode::OK, Json(Some(sig)))
        })
}

/// Get all pending signatures
async fn list_signatures_handler(State(state): State<SignatureApiState>) -> impl IntoResponse {
    let signatures = state.store.pending_signatures();
    Json(signatures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::signer::MessageSignature;
    use alloy::primitives::Address;
    use axum::{body::Body, http::Request};
    use tower::util::ServiceExt;

    fn mock_signature(id: &str) -> MessageSignature {
        MessageSignature {
            message_id: id.to_string(),
            signature: vec![0u8; 65],
            signer: Address::ZERO,
            signed_at: 1700000000,
        }
    }

    fn create_test_app() -> (Router, SignatureStore) {
        let store = SignatureStore::new();
        let state = SignatureApiState::new(
            store.clone(),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            1,
        );
        let router = create_signature_router(state);
        (router, store)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let (router, _store) = create_test_app();

        let response = router
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_info_endpoint() {
        let (router, _store) = create_test_app();

        let response = router
            .oneshot(Request::get("/info").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: SignerInfoResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(info.signer, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        assert_eq!(info.chain_id, 1);
    }

    #[tokio::test]
    async fn test_pending_empty() {
        let (router, _store) = create_test_app();

        let response = router
            .oneshot(Request::get("/pending").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let pending: PendingResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(pending.count, 0);
        assert!(pending.message_ids.is_empty());
    }

    #[tokio::test]
    async fn test_pending_with_signatures() {
        let (router, store) = create_test_app();

        store.store(mock_signature("0x1111"));
        store.store(mock_signature("0x2222"));

        let response = router
            .oneshot(Request::get("/pending").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let pending: PendingResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(pending.count, 2);
    }

    #[tokio::test]
    async fn test_get_signature_found() {
        let (router, store) = create_test_app();

        store.store(mock_signature("0x1234"));

        let response = router
            .oneshot(
                Request::get("/signature/0x1234")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let sig: MessageSignature = serde_json::from_slice(&body).unwrap();

        assert_eq!(sig.message_id, "0x1234");
    }

    #[tokio::test]
    async fn test_get_signature_not_found() {
        let (router, _store) = create_test_app();

        let response = router
            .oneshot(
                Request::get("/signature/0xnotfound")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_signatures() {
        let (router, store) = create_test_app();

        store.store(mock_signature("0x1111"));
        store.store(mock_signature("0x2222"));
        store.mark_submitted("0x1111"); // This one won't be in pending

        let response = router
            .oneshot(Request::get("/signatures").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let sigs: Vec<MessageSignature> = serde_json::from_slice(&body).unwrap();

        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].message_id, "0x2222");
    }
}
