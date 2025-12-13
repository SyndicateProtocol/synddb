//! Shared types for `SyndDB` wire format
//!
//! This module contains the core types used for communication between
//! `synddb-sequencer`, `synddb-validator`, and `synddb-client`.
//!
//! All wire format uses CBOR/COSE binary encoding. Types are organized in submodules:
//! - `cbor`: CBOR/COSE binary format types (primary wire format)
//! - `message`: Internal message types after parsing from CBOR
//! - `payloads`: Request/response payload types for HTTP API

pub mod cbor;
pub mod message;
pub mod payloads;
