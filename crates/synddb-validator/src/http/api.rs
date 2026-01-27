//! HTTP API endpoints for validator health and status
//!
//! This module provides operational endpoints for monitoring validator state.
//! Unlike the sequencer's HTTP API, the validator API is read-only and provides
//! health checks and synchronization status.
//!
//! # Endpoints
//!
//! | Endpoint | Method | Description |
//! |----------|--------|-------------|
//! | `/health` | GET | Liveness probe (always returns 200 OK) |
//! | `/healthz` | GET | Alias for `/health` (Kubernetes convention) |
//! | `/ready` | GET | Readiness probe (503 if not running) |
//! | `/status` | GET | Sync status and last sequence number |
//!
//! # Status Response
//!
//! The `/status` endpoint returns JSON:
//!
//! ```json
//! {
//!   "running": true,
//!   "last_sequence": 42,
//!   "last_sync_time": 1700000000
//! }
//! ```
//!
//! - `last_sequence` is `null` if the validator has never synced
//! - `last_sync_time` is a Unix timestamp (seconds)

use axum::{
    extract::State,
    http::{header::HeaderName, Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use axum_prometheus::PrometheusMetricLayer;
use metrics_exporter_prometheus::PrometheusHandle;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tower_http::{
    request_id::{PropagateRequestIdLayer, SetRequestIdLayer},
    trace::{DefaultOnResponse, MakeSpan, TraceLayer},
};
use tracing::{Level, Span};

use super::request_id::UuidRequestId;

/// Shared application state for HTTP handlers
///
/// Uses atomic values for thread-safe access from HTTP handlers.
#[derive(Clone, Debug)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    /// Track if validator is running
    running: AtomicBool,
    /// Last synced sequence number (`u64::MAX` means never synced)
    last_sequence: AtomicU64,
    /// Last sync timestamp (Unix seconds)
    last_sync_time: AtomicU64,
}

impl std::fmt::Debug for AppStateInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppStateInner")
            .field("running", &self.running.load(Ordering::SeqCst))
            .field("last_sequence", &self.last_sequence.load(Ordering::SeqCst))
            .field(
                "last_sync_time",
                &self.last_sync_time.load(Ordering::SeqCst),
            )
            .finish()
    }
}

/// Sentinel value for "never synced"
const NOT_SYNCED: u64 = u64::MAX;

impl AppState {
    /// Create new application state
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                running: AtomicBool::new(false),
                last_sequence: AtomicU64::new(NOT_SYNCED),
                last_sync_time: AtomicU64::new(0),
            }),
        }
    }

    /// Set the running state
    pub fn set_running(&self, running: bool) {
        self.inner.running.store(running, Ordering::SeqCst);
    }

    /// Get the running state
    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    /// Update sync status (called by sync loop)
    pub fn update_sync_status(&self, last_sequence: Option<u64>, last_sync_time: u64) {
        let seq = last_sequence.unwrap_or(NOT_SYNCED);
        self.inner.last_sequence.store(seq, Ordering::SeqCst);
        self.inner
            .last_sync_time
            .store(last_sync_time, Ordering::SeqCst);
    }

    /// Get the last synced sequence
    fn last_sequence(&self) -> Option<u64> {
        let seq = self.inner.last_sequence.load(Ordering::SeqCst);
        if seq == NOT_SYNCED {
            None
        } else {
            Some(seq)
        }
    }

    /// Get the last sync time
    fn last_sync_time(&self) -> u64 {
        self.inner.last_sync_time.load(Ordering::SeqCst)
    }

    /// Get the next sequence to sync
    fn next_sequence(&self) -> u64 {
        self.last_sequence().map_or(0, |s| s + 1)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Health check response
#[derive(Debug, Serialize, Deserialize)]
struct HealthResponse {
    status: String,
    running: bool,
}

/// Status response with sync progress
#[derive(Debug, Serialize, Deserialize)]
struct StatusResponse {
    running: bool,
    last_sequence: Option<u64>,
    last_sync_time: u64,
    next_sequence: u64,
}

/// Custom span maker that includes request ID in all spans
#[derive(Clone, Debug)]
struct RequestIdMakeSpan;

impl<B> MakeSpan<B> for RequestIdMakeSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        let request_id = request
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");

        tracing::info_span!(
            "http_request",
            method = %request.method(),
            uri = %request.uri(),
            request_id = %request_id,
        )
    }
}

/// Create the HTTP router
pub fn create_router(state: AppState, metrics_handle: PrometheusHandle) -> Router {
    // Create HTTP metrics layer (uses existing global recorder from init_metrics)
    let prometheus_layer = PrometheusMetricLayer::new();

    // Create metrics endpoint handler
    let metrics_endpoint = synddb_shared::metrics::metrics_endpoint(metrics_handle);

    // Request ID header name
    let x_request_id = HeaderName::from_static("x-request-id");

    Router::new()
        .route("/health", get(health_handler))
        .route("/healthz", get(health_handler))
        .route("/status", get(status_handler))
        .route("/ready", get(ready_handler))
        .route("/metrics", get(metrics_endpoint))
        // Layers are applied in reverse order (bottom to top)
        .layer(PropagateRequestIdLayer::new(x_request_id.clone()))
        .layer(prometheus_layer)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(RequestIdMakeSpan)
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(SetRequestIdLayer::new(x_request_id, UuidRequestId))
        .with_state(state)
}

/// Health check endpoint (always healthy if server is running)
async fn health_handler(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        running: state.is_running(),
    })
}

/// Ready check endpoint (returns 503 if not running)
async fn ready_handler(State(state): State<AppState>) -> impl IntoResponse {
    let running = state.is_running();

    if running {
        (
            StatusCode::OK,
            Json(HealthResponse {
                status: "ready".to_string(),
                running,
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "not_ready".to_string(),
                running,
            }),
        )
    }
}

/// Status endpoint with sync progress details
async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(StatusResponse {
        running: state.is_running(),
        last_sequence: state.last_sequence(),
        last_sync_time: state.last_sync_time(),
        next_sequence: state.next_sequence(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::util::ServiceExt;

    fn create_test_app() -> (Router, AppState) {
        let app_state = AppState::new();
        let metrics_handle = synddb_shared::metrics::init_metrics();
        let router = create_router(app_state.clone(), metrics_handle);
        (router, app_state)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let (router, _state) = create_test_app();

        let response = router
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let health: HealthResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(health.status, "healthy");
        assert!(!health.running);
    }

    #[tokio::test]
    async fn test_healthz_endpoint() {
        let (router, _state) = create_test_app();

        let response = router
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_ready_not_running() {
        let (router, _state) = create_test_app();

        let response = router
            .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let health: HealthResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(health.status, "not_ready");
    }

    #[tokio::test]
    async fn test_ready_when_running() {
        let (router, state) = create_test_app();
        state.set_running(true);

        let response = router
            .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let health: HealthResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(health.status, "ready");
        assert!(health.running);
    }

    #[tokio::test]
    async fn test_status_endpoint() {
        let (router, _state) = create_test_app();

        let response = router
            .oneshot(Request::get("/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: StatusResponse = serde_json::from_slice(&body).unwrap();

        assert!(!status.running);
        assert_eq!(status.last_sequence, None);
        assert_eq!(status.next_sequence, 0);
    }

    #[tokio::test]
    async fn test_status_after_sync() {
        let (router, state) = create_test_app();

        // Simulate some sync progress
        state.update_sync_status(Some(42), 1700000000);
        state.set_running(true);

        let response = router
            .oneshot(Request::get("/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: StatusResponse = serde_json::from_slice(&body).unwrap();

        assert!(status.running);
        assert_eq!(status.last_sequence, Some(42));
        assert_eq!(status.last_sync_time, 1700000000);
        assert_eq!(status.next_sequence, 43);
    }
}
