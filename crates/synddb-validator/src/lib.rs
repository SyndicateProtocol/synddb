//! `SyndDB` Validator - validates sequencer messages and applies changesets
//!
//! The validator syncs state from DA layers, verifies sequencer signatures,
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
//!
//! # TODO: Metrics/Observability
//!
//! The following metrics should be exposed via a Prometheus endpoint:
//!
//! - `synddb_validator_messages_synced_total` - Counter of synced messages
//! - `synddb_validator_last_sequence` - Gauge of last synced sequence number
//! - `synddb_validator_sync_lag_seconds` - Gauge of time since last sync
//! - `synddb_validator_changeset_apply_duration_seconds` - Histogram of apply times
//! - `synddb_validator_signature_verify_duration_seconds` - Histogram of verify times
//! - `synddb_validator_gaps_detected_total` - Counter of detected gaps
//! - `synddb_validator_withdrawals_signed_total` - Counter of signed withdrawals

pub mod apply;
pub mod bridge;
pub mod config;
pub mod error;
pub mod http;
pub mod state;
pub mod sync;
pub mod validator;

// Re-export main types for convenience
pub use apply::ChangesetApplier;
pub use bridge::{BridgeSigner, MessageSignature, SignatureStore};
pub use config::ValidatorConfig;
pub use error::ValidatorError;
pub use http::{create_router, create_signature_router, AppState, SignatureApiState};
pub use state::StateStore;
pub use sync::{DAFetcher, SignatureVerifier};
pub use validator::Validator;
