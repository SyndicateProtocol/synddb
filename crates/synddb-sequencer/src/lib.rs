//! `SyndDB` Sequencer - Message Ordering and Signing Service
//!
//! The sequencer receives messages (changesets, withdrawals) from synddb-client
//! applications, assigns monotonic sequence numbers, and signs them with a
//! key generated inside the TEE at startup.
//!
//! # Security Model
//!
//! The signing key is generated fresh at startup inside the TEE using secure
//! OS-level randomness. The private key material never leaves the enclave and
//! is never logged. Only the public key and derived address are exposed for
//! external verification.
//!
//! # Architecture
//!
//! ```text
//! synddb-client (App TEE)
//!        │
//!        │ HTTP POST /changesets
//!        ▼
//! ┌─────────────────────────┐
//! │   synddb-sequencer      │
//! │   (Sequencer TEE)       │
//! │                         │
//! │  ┌─────────────────┐    │
//! │  │   HTTP API      │    │
//! │  └────────┬────────┘    │
//! │           │             │
//! │  ┌────────▼────────┐    │
//! │  │     Inbox       │    │
//! │  │  (Sequencing)   │    │
//! │  └────────┬────────┘    │
//! │           │             │
//! │  ┌────────▼────────┐    │
//! │  │  EvmKeyManager  │    │
//! │  │  (secp256k1)    │    │
//! │  └────────┬────────┘    │
//! │           │             │
//! │  ┌────────▼────────┐    │
//! │  │   Transport     │    │
//! │  │  (GCS, etc.)    │    │
//! │  └─────────────────┘    │
//! └─────────────────────────┘
//! ```
//!
//! # Usage
//!
//! Run the sequencer (key is generated automatically):
//!
//! ```bash
//! synddb-sequencer
//! ```
//!
//! With GCS persistence:
//!
//! ```bash
//! GCS_BUCKET=my-bucket synddb-sequencer
//! ```

pub mod attestation;
pub mod batcher;
pub mod cbor_extractor;
pub mod config;
pub mod http_api;
pub mod http_errors;
pub mod inbox;
pub mod messages;
pub mod transport;
