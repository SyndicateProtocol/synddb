//! `SyndDB` Validator - validates sequencer messages and applies changesets
//!
//! The validator syncs state from storage layers, verifies sequencer signatures,
//! and applies `SQLite` changesets to maintain a replica of the sequenced state.
//!
//! # Features
//!
//! - **Signature Verification**: Verifies that messages are signed by the expected sequencer
//! - **Changeset Application**: Applies `SQLite` changesets to reconstruct state
//! - **Snapshot Restoration**: Fast bootstrap from database snapshots
//! - **State Persistence**: Tracks sync progress for crash recovery
//! - **Gap Detection**: Detects and handles missing sequence numbers
//! - **HTTP API**: Provides health checks and status endpoints
//! - **Bridge Signing**: Optional mode to sign messages for the bridge contract

pub mod apply;
pub mod bridge;
pub mod config;
pub mod error;
pub mod http;
pub mod metrics;
pub mod rules;
pub mod state;
pub mod sync;
pub mod validator;
