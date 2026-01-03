//! Request ID generation and propagation for distributed tracing
//!
//! This module provides middleware for generating and propagating request IDs
//! across HTTP requests. Request IDs enable correlation of logs and traces
//! across services in Google Cloud Logging and Cloud Trace.

use axum::http::{HeaderValue, Request};
use tower_http::request_id::{MakeRequestId, RequestId};
use uuid::Uuid;

/// Request ID generator using UUID v4
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
