//! HTTP API module for validator health checks and status
//!
//! This module provides two HTTP APIs:
//! - Main API (default port 8080): Health checks, status, and sync progress
//! - Signature API (default port 8081): Bridge signature retrieval for relayers

mod api;
mod signatures;

pub use api::{create_router, AppState};
pub use signatures::{create_signature_router, SignatureApiState};
