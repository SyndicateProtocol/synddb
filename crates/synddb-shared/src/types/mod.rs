//! Shared types for Message Passing Bridge wire format
//!
//! This module contains the core types used for communication between
//! validators and the Bridge contract.
//!
//! All wire format uses CBOR/COSE binary encoding. Types are organized in submodules:
//! - `cbor`: CBOR/COSE binary format types (primary wire format)
//! - `message`: Internal message types after parsing from CBOR
//! - `payloads`: Request/response payload types for HTTP API

pub mod cbor;
pub mod message;
pub mod payloads;
