//! `SyndDB` Sequencer - Message Ordering and Signing Service
//!
//! The sequencer receives messages (changesets, withdrawals) from synddb-client
//! applications, assigns monotonic sequence numbers, and signs them with a
//! private key.
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
//! │  │    Signer       │    │
//! │  │  (secp256k1)    │    │
//! │  └────────┬────────┘    │
//! │           │             │
//! │  ┌────────▼────────┐    │
//! │  │   Publisher     │    │
//! │  │  (GCS, etc.)    │    │
//! │  └─────────────────┘    │
//! └─────────────────────────┘
//! ```
//!
//! # Usage
//!
//! Run the sequencer with a signing key:
//!
//! ```bash
//! SIGNING_KEY=<hex-private-key> synddb-sequencer
//! ```
//!
//! With GCS persistence:
//!
//! ```bash
//! SIGNING_KEY=<key> GCS_BUCKET=my-bucket synddb-sequencer
//! ```

pub mod attestation;
pub mod config;
pub mod http_api;
pub mod inbox;
pub mod publish;
pub mod signer;
