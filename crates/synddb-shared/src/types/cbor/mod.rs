//! CBOR/COSE types for binary batch serialization
//!
//! This module provides efficient binary serialization using CBOR encoding
//! with `COSE_Sign1` signatures for message authentication.
//!
//! # Structure
//!
//! - `CborBatch`: Container for multiple signed messages
//! - `CborSignedMessage`: Individual message wrapped in `COSE_Sign1`
//! - `CborMessageType`: Message type enum (compact integer encoding)
//!
//! # Example
//!
//! ```rust,ignore
//! use synddb_shared::types::cbor::batch::CborBatch;
//! use synddb_shared::types::cbor::message::{CborMessageType, CborSignedMessage};
//!
//! // Create a signed message
//! let msg = CborSignedMessage::new(
//!     sequence,
//!     timestamp,
//!     CborMessageType::Changeset,
//!     payload,
//!     signer_address,
//!     |data| sign(data),
//! )?;
//!
//! // Create a batch
//! let batch = CborBatch::new(
//!     vec![msg],
//!     created_at,
//!     signer_address,
//!     |data| sign(data),
//! )?;
//!
//! // Serialize to compressed CBOR
//! let bytes = batch.to_cbor_zstd()?;
//!
//! // Parse and verify
//! let batch = CborBatch::from_cbor_zstd(&bytes)?;
//! batch.verify_all_signatures()?;
//!
//! // Debug as JSON
//! println!("{}", batch.to_json_pretty()?);
//! ```

pub mod batch;
pub mod convert;
pub mod cose_helpers;
pub mod debug;
pub mod error;
pub mod message;
