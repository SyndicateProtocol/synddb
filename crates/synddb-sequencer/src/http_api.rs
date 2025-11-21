//! HTTP API for receiving changesets from application TEE

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tracing::{info, warn};

use crate::monitor::Changeset;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetBatch {
    pub changesets: Vec<Changeset>,
    pub batch_id: String,
}

#[derive(Clone, Debug)]
pub struct ApiState {
    pub changeset_tx: Sender<Changeset>,
}

pub fn create_router(state: ApiState) -> Router {
    Router::new()
        .route("/changesets", post(receive_changesets))
        .route("/health", axum::routing::get(health_check))
        .with_state(state)
}

/// Receive changesets from application TEE
async fn receive_changesets(
    State(state): State<ApiState>,
    Json(batch): Json<ChangesetBatch>,
) -> Result<Json<ReceiveResponse>, StatusCode> {
    info!(
        "Received changeset batch: {} changesets, batch_id={}",
        batch.changesets.len(),
        batch.batch_id
    );

    // TODO: Validate TEE attestation of sender
    // This ensures only authorized application TEEs can submit changesets

    // Forward changesets to processing pipeline
    for changeset in batch.changesets {
        if let Err(e) = state.changeset_tx.send(changeset).await {
            warn!("Failed to forward changeset: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    Ok(Json(ReceiveResponse {
        status: "accepted".to_string(),
        batch_id: batch.batch_id,
    }))
}

async fn health_check() -> &'static str {
    "OK"
}

#[derive(Debug, Serialize)]
struct ReceiveResponse {
    status: String,
    batch_id: String,
}
