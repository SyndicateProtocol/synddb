//! HTTP API module for validator health checks and status
//!
//! This module provides two HTTP APIs:
//! - Main API (default port 8080): Health checks, status, and sync progress
//! - Signature API (default port 8081): Bridge signature retrieval for relayers

pub mod api;
pub mod request_id;
pub mod signatures;
