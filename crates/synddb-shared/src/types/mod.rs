//! Shared types for `SyndDB` wire format
//!
//! This module contains the core types used for communication between
//! `synddb-sequencer`, `synddb-validator`, and `synddb-client`.
//!
//! Types are organized in submodules:
//! - `cbor`: CBOR/COSE binary format types (primary format)
//! - `message`: Legacy JSON message types (being replaced)
//! - `payloads`: Request/response payload types
//! - `serde_helpers`: Serialization utilities (e.g., base64)

pub mod cbor;
pub mod message;
pub mod payloads;
pub mod serde_helpers;
