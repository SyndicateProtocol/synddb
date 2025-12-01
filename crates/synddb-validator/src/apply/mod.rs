//! Changeset application module
//!
//! Applies `SQLite` changesets from sequenced messages to reconstruct state.

mod applier;

pub use applier::ChangesetApplier;
