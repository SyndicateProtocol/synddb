//! Shared utilities and types for `SyndDB`
//!
//! This crate provides common types and utilities used across the `SyndDB` ecosystem:
//!
//! - [`gcs`]: Google Cloud Storage configuration
//! - [`parse`]: CLI argument parsing helpers
//! - [`prelude`]: Convenient re-exports of common types
//! - [`runtime`]: Tokio runtime utilities
//! - [`types`]: Wire format types (messages, batches, payloads)
//!
//! # Quick Start
//!
//! Use the prelude for convenient access to common types:
//!
//! ```rust
//! use synddb_shared::prelude::*;
//! ```

pub mod gcs;
pub mod parse;
pub mod prelude;
pub mod runtime;
pub mod types;
