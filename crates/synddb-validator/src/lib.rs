//! `SyndDB` Validator - validates sequencer messages and applies changesets
//!
//! The validator syncs state from DA layers, verifies sequencer signatures,
//! and applies `SQLite` changesets to maintain a replica of the sequenced state.
//!
//! # Features
//!
//! - **Signature Verification**: Verifies that messages are signed by the expected sequencer
//! - **Changeset Application**: Applies `SQLite` changesets to reconstruct state
//! - **State Persistence**: Tracks sync progress for crash recovery
//! - **HTTP API**: Provides health checks and status endpoints

pub mod apply;
pub mod config;
pub mod error;
pub mod http;
pub mod state;
pub mod sync;
pub mod validator;

// Re-export main types for convenience
pub use apply::ChangesetApplier;
pub use config::ValidatorConfig;
pub use error::ValidatorError;
pub use http::{create_router, AppState};
pub use state::StateStore;
pub use sync::{DAFetcher, SignatureVerifier};
pub use validator::Validator;
