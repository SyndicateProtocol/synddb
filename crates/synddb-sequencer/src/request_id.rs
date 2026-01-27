//! Request ID generation and propagation for distributed tracing
//!
//! This module provides middleware for generating and propagating request IDs
//! across HTTP requests. Request IDs enable correlation of logs and traces
//! across services in Google Cloud Logging and Cloud Trace.
//!
//! # Headers
//!
//! - `x-request-id`: The primary request ID header. If provided by the client,
//!   it will be preserved. Otherwise, a new UUID v4 will be generated.
//!
//! # Usage
//!
//! The middleware layers should be applied in this order:
//! 1. `SetRequestIdLayer` - generates request ID if not present
//! 2. `TraceLayer` - creates span with request ID
//! 3. `PropagateRequestIdLayer` - copies request ID to response

use axum::http::{HeaderValue, Request};
use tower_http::request_id::{MakeRequestId, RequestId};
use uuid::Uuid;

/// Request ID generator using UUID v4
///
/// This generates a new UUID v4 for each request that doesn't already
/// have a request ID header.
#[derive(Clone, Copy, Default, Debug)]
pub struct UuidRequestId;

impl MakeRequestId for UuidRequestId {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let uuid = Uuid::new_v4();
        let header_value =
            HeaderValue::from_str(&uuid.to_string()).expect("UUID is always valid header value");
        Some(RequestId::new(header_value))
    }
}

/// Extract the request ID from a request, if present
pub fn extract_request_id<B>(request: &Request<B>) -> Option<String> {
    request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}
